//! 磁盘数据结构层的代码在 layout.rs 和 bitmap.rs 中
//!
//! 在 fs 布局中存在两类不同的位图, 分别对索引节点和数据块进行管理
//!
//! 每个位图都由若干个块组成, 每个块大小为 512 bytes, 即 4096 bits
//! 每个 bit 都代表一个索引节点/数据块的分配状态,  0 意味着未分配, 而 1 则意味着已经分配出去
//!
//! 位图所要做的事情是通过基于 bit 为单位的分配(寻找一个为 0 的 bit 位并设置为 1)
//! 和回收(将bit位清零)来进行索引节点/数据块的分配和回收

use std::sync::Arc;

use super::{get_block_cache, BlockDevice, BLOCK_BITS};

/// 磁盘块上位图区域的数据以磁盘数据结构 BitmapBlock 的格式进行操作.
/// BitmapBlock 是一个磁盘数据结构, 它将位图区域中的一个磁盘块解释为长度为 64 的一个 u64 数组,
/// 每个 u64 打包了一组 64 bits, 于是整个数组包含 64 * 64 = 4096 bits, 且可以以组为单位进行操作
/// 刚好占用一个磁盘块的大小.
type BitmapBlock = [u64; 64]; // size = 64 * 64 = 4096 bits = 512 bytes

/// Bitmap 自身是驻留在内存中的,
/// 但是它能够表示索引节点/数据块区域中的那些磁盘块的分配情况.
pub struct Bitmap {
    /// 位图所在区域的起始块编号
    start_block_id: usize,
    /// 位图索引使用的磁盘块数
    blocks_counts: usize,
}

impl Bitmap {
    pub fn new(start_block_id: usize, blocks_counts: usize) -> Self {
        Self {
            start_block_id,
            blocks_counts,
        }
    }

    /// 从块设备分配一个新块
    ///
    /// 遍历区域中的每个块,
    /// 再在每个块中以 bit 组(每组 64 bits)为单位进行遍历,
    /// 找到一个尚未被全部分配出去的组,
    /// 最后在里面分配一个 bit.
    ///
    /// 它将会返回分配的 bit 所在的位置, 等同于 索引节点/数据块 的编号.
    ///
    /// 如果所有bit均已经被分配出去了, 则返回 None .
    pub fn alloc(&self, block_device: &Arc<dyn BlockDevice>) -> Option<usize> {
        // 枚举区域中的每个块(编号为 block_id ), 在循环内部我们需要读写这个块, 在块内尝试找到一个空闲的bit并置 1 .
        // 一旦涉及到块的读写, 就需要用到块缓存层提供的接口
        for block_id in 0..self.blocks_counts {
            // 调用 get_block_cache 获取块缓存
            let pos = get_block_cache(
                // 注意传入的块编号是区域起始块编号 start_block_id 加上区域内的块编号 block_id 得到的块设备上的块编号
                block_id + self.start_block_id as usize,
                Arc::clone(block_device),
            )
            // 通过 .lock() 获取块缓存的互斥锁从而可以对块缓存进行访问
            .lock()
            // 使用 BlockCache::modify 接口.
            //
            // 它传入的偏移量 offset 为 0, 这是因为整个块上只有一个 BitmapBlock , 它的大小恰好为 512 字节, (see BlockCache.cache which is a [u8; BLOCK_SZ])
            // 因此我们需要从块的开头开始才能访问到完整的 BitmapBlock .
            //
            // 同时, 传给它的闭包需要显式声明参数类型为 &mut BitmapBlock ,
            // 不然的话,  BlockCache 的泛型方法 modify/get_mut 无法得知应该用哪个类型来解析块上的数据.
            // 在声明之后, 编译器才能在这里将两个方法中的泛型 T 实例化为具体类型 BitmapBlock .
            //
            // 总结一下, 这里 modify 的含义就是:
            // 从缓冲区偏移量为 0 的位置开始将一段连续的数据(数据的长度随具体类型而定)解析为一个 BitmapBlock 并要对该数据结构进行修改.
            // 在闭包内部, 我们可以使用这个 BitmapBlock 的可变引用 bitmap_block 对它进行访问.
            // read/get_ref 的用法完全相同, 后面将不再赘述.
            .modify(0, |bitmap_block: &mut BitmapBlock| -> Option<usize> {
                // 返回值赋值给 pos

                // 尝试在 bitmap_block 中找到一个空闲的 bit 并返回其位置.
                // 如果能够找到的话, bit 组的编号将保存在变量 bits64_pos 中, 而分配的 bit 在组内的位置将保存在变量 inner_pos 中.
                // bits64_pos: 为 bitmap_block 数组的某元素 (bits64) 的下标 (bits64_pos/bitmap_index), 该元素以二进制解释不是全 1
                // inner_pos: 范围 [0, 63], 该元素以二进制解释时最左边的(最低位的) 0 的位置
                if let Some((bits64_pos, inner_pos)) = bitmap_block
                    // 遍历每 64 bits构成的组(一个 u64 )
                    .iter()
                    .enumerate()
                    // 如果它并没有达到 u64::MAX (不是 0x1111..1111, 即该行未分配完),
                    .find(|(_, bits64)| **bits64 != u64::MAX)
                    // 则通过 u64::trailing_ones 找到最低的一个 0 的位置(从第 0 位开始计算)
                    .map(|(bits64_pos, bits64)| (bits64_pos, bits64.trailing_ones() as usize))
                // 在此处返回 if let 匹配的bits64_pos, inner_pos = bits64.trailing_ones()
                {
                    // 或运算 将该位置置为 1
                    bitmap_block[bits64_pos] |= 1 << inner_pos;

                    // 在返回分配的 bit 编号的时候, 它的计算方式是:
                    // block_id(块号) * BLOCK_BITS(每块大小: bits) + bits64_pos(行号, 块内组号, 数组index) * 64 + inner_pos(组内编号, 最低位的 0 的位置(已经修改为 1 ))
                    Some(block_id * BLOCK_BITS + bits64_pos * 64 + inner_pos as usize)

                    // 返回值赋值给变量 pos

                    // 注意闭包中的 block_id 并不在闭包的参数列表中, 因此它是从外部环境(即自增 block_id 的循环)中捕获到的.
                    //
                    // Rust 语法卡片: 闭包
                    //
                    // 闭包是持有外部环境变量的函数.
                    // 所谓外部环境, 就是指创建闭包时所在的词法作用域.
                    // Rust中定义的闭包, 按照对外部环境变量的使用方式(借用, 复制, 转移所有权), 分为三个类型: Fn, FnMut, FnOnce.
                    // Fn类型的闭包会在闭包内部以共享借用的方式使用环境变量;
                    // FnMut类型的闭包会在闭包内部以独占借用的方式使用环境变量;
                    // 而FnOnce类型的闭包会在闭包内部以所有者的身份使用环境变量.
                    // 根据闭包内使用环境变量的方式, 即可判断创建出来的闭包的类型.
                } else {
                    // 如果所有bit均已经被分配出去了, 则返回 None
                    None
                }
            });
            // 一旦在某个块中找到一个空闲的bit并成功分配, 就不再考虑后续的块, 提前返回
            if pos.is_some() {
                return pos;
            }
        }
        None
    }

    pub fn dealloc(&self, block_device: &Arc<dyn BlockDevice>, bit: usize) {
        let (block_id, bits64_pos, inner_pos) = decomposition(bit);
        get_block_cache(
            block_id + self.start_block_id as usize,
            Arc::clone(block_device),
        )
        .lock()
        .modify(0, |bitmap_block: &mut BitmapBlock| {
            assert!(bitmap_block[bits64_pos] & (1 << inner_pos) != 0);
            bitmap_block[bits64_pos] &= !(1u64 << inner_pos);
        });
    }

    /// 获取可分配块的最大数量
    pub fn maximum(&self) -> usize {
        self.blocks_counts * BLOCK_BITS
    }
}

/// 将bit编号 bit 分解为区域中的块编号 block_pos , 块内的组编号 bits64_pos 以及组内编号 inner_pos 的三元组
fn decomposition(mut bit: usize) -> (usize, usize, usize) {
    let block_id = bit / BLOCK_BITS;
    bit %= BLOCK_BITS;
    (block_id, bit / 64, bit % 64)
}
