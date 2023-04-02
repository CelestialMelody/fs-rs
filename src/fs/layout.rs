//! 磁盘数据结构层的代码在 layout.rs 和 bitmap.rs 中
//!
//! 三个数据结构 [`SuperBlock`], [`DiskInode`], [`DirEntry`]
//!
//! 在 fs 磁盘布局中, 按照块编号从小到大顺序地分成 5 个不同属性的连续区域:
//!
//! - 最开始的区域的长度为一个块, 其内容是超级块 ([`SuperBlock`])
//!   超级块内以 魔数 的形式提供了文件系统合法性检查功能, 同时还可以定位其他连续区域的位置
//!
//! - 第二个区域是一个索引节点位图, 长度为若干个块
//!   它记录了后面的索引节点区域中有哪些索引节点已经被分配出去使用了, 而哪些还尚未被分配出去
//!
//! - 第三个区域是索引节点区域, 长度为若干个块
//!   其中的每个块都存储了若干个索引节点
//!
//! - 第四个区域是一个数据块位图, 长度为若干个块
//!   它记录了后面的数据块区域中有哪些数据块已经被分配出去使用了, 而哪些还尚未被分配出去.
//!
//! - 最后的区域则是数据块区域
//!   其中的每一个已经分配出去的块保存了文件或目录中的具体数据内容.

use std::{
    fmt::{Debug, Formatter, Result},
    sync::Arc,
};

use super::{
    get_block_cache, BlockDevice, BLOCK_SIZE, DIRENT_SIZE, EAZY_FS_MAGIC, INDIRECT1_BOUND,
    INODE_DIRECT_COUNT, INODE_INDIRECT1_COUNT, INODE_INDIRECT2_COUNT, NAME_LENGTH_LIMIT,
};

#[repr(C)]
pub struct SuperBlock {
    magic: u32, // 用于文件系统合法性验证的魔数
    pub total_blocks: u32,
    pub inode_bitmap_blocks: u32,
    pub inode_area_blocks: u32,
    pub data_bitmap_blocks: u32,
    pub data_area_blocks: u32,
}

impl Debug for SuperBlock {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        f.debug_struct("SuperBlock")
            .field("magic", &self.magic)
            .field("total_blocks", &self.total_blocks)
            .field("inode_bitmap_blocks", &self.inode_bitmap_blocks)
            .field("inode_area_blocks", &self.inode_area_blocks)
            .field("data_bitmap_blocks", &self.data_bitmap_blocks)
            .field("data_area_blocks", &self.data_area_blocks)
            .finish()
    }
}

/// SuperBlock 是一个磁盘上数据结构, 它就存放在磁盘上编号为 0 的块的起始处
impl SuperBlock {
    /// 创建一个 fs 的时候对超级块进行初始化,
    /// 注意, 各个区域的块数是以参数的形式传入进来的,
    /// 它们的划分是更上层的 磁盘块管理器 需要完成的工作
    pub fn initialize(
        &mut self,
        total_blocks: u32,
        inode_bitmap_blocks: u32,
        inode_area_blocks: u32,
        data_bitmap_blocks: u32,
        data_area_blocks: u32,
    ) {
        *self = Self {
            magic: EAZY_FS_MAGIC,
            total_blocks,
            inode_bitmap_blocks,
            inode_area_blocks,
            data_bitmap_blocks,
            data_area_blocks,
        };
    }

    /// is_valid 可以通过魔数判断超级块所在的文件系统是否合法
    pub fn is_valid(&self) -> bool {
        self.magic == EAZY_FS_MAGIC
    }
}

#[derive(PartialEq, Debug)]
pub enum DiskInodeType {
    File,
    Directory,
}

/// 索引块 IndirectBlock 实质上是一个 u32 数组, 每个都指向一个下一级索引块或者数据块
type IndirectBlock = [u32; BLOCK_SIZE / 4]; // size = 512B / 4B(u32) = 128

// 作为一个文件而言, 它的内容在文件系统看来没有任何既定的格式, 都只是
// 一个 (u8) 字节序列, 因此每个保存内容的数据块都只是一个字节数组
type DataBlock = [u8; BLOCK_SIZE]; // size = 512B

/// 每个 文件/目录 在磁盘上均以一个 DiskInode 的形式存储
///
/// 由于字节对齐, DiskInode 大小为 (1 + 28 + 1 + 1) * 4 + 4(字节对齐) = 128 B
///
/// 为了充分利用空间, 将 DiskInode 的大小设置为 128 字节, 每个块正好能够容纳 4 个 DiskInode
//
// 注意: 在后续需要支持更多类型的元数据的时候, 可以适当缩减直接索引 direct 的块
// 数, 并将节约出来的空间用来存放其他元数据, 仍可保证 DiskInode 的总大小为 128 字节
//
// Q: 删除文件 / 文件夹时如何删除索引节点块中的索引节点?
// 由于一个块中可以存放 4 个索引节点, 因此相较于删除数据节点, 删除索引节点没那么容易 (可能需要修改数据结构)
#[repr(C)]
pub struct DiskInode {
    /// 文件/目录内容的字节数
    pub size: u32,
    /// 已经分配的大小, 并不一定等于 size
    pub alloc_size: u32,
    /// 直接索引块(号组)
    ///
    /// 当文件很小的时候, 只需用到直接索引, direct 数组中最多可以指向 INODE_DIRECT_COUNT 个数据块
    ///
    /// 通过直接索引可以找到 BLOCK_SZ * INODE_DIRECT_COUNT 的内容
    pub direct: [u32; INODE_DIRECT_COUNT],
    /// 一级间接索引块(号)
    ///
    /// 当文件比较大的时候, 不仅直接索引的 direct 数组装满, 还需要用到一级间接索引 indirect1
    /// , 它指向一个一级索引块, 这个块也位于磁盘布局的数据块区域中
    /// . 这个一级索引块中的每个 u32 都用来指向数据块区域中一个保存该文件内容的数据块
    /// . 因此, 最多能够索引 INODE_INDIRECT1_COUNT =  512B / 4B(u32) = 128 数据块, 对应 512B * 128 = 64KB 的内容
    pub indirect1: u32,
    /// 二级间接索引块(号)
    ///
    /// 当文件大小超过直接索引和一级索引支持的容量上限的时候, 就需要用到二级间接索引 indirect2
    /// . 它指向一个位于数据块区域中的二级索引块. 二级索引块中的每个 u32 指向
    /// 一个不同的一级索引块, 这些一级索引块也位于数据块区域中
    /// . 因此, 通过二级间接索引最多能够索引 128 * 64KB = 8MB 的内容
    pub indirect2: u32,
    /// 索引节点的类型 DiskInodeType, 目前仅支持文件 File 和目录 Directory 两种类型
    pub type_: DiskInodeType,
}

impl DiskInode {
    pub fn initialize(&mut self, type_: DiskInodeType) {
        self.size = 0;
        self.alloc_size = 0;
        self.direct.iter_mut().for_each(|x| *x = 0);
        self.indirect1 = 0;
        self.indirect2 = 0;
        self.type_ = type_;
    }

    pub fn is_dir(&self) -> bool {
        self.type_ == DiskInodeType::Directory
    }

    pub fn is_file(&self) -> bool {
        self.type_ == DiskInodeType::File
    }

    /// 通过索引查到它自身用于保存文件内容的第 block_id 个数据块的块编号, 这样后续才能对这个数据块进行访问
    pub fn get_block_id(&self, inner_id: u32, block_device: &Arc<dyn BlockDevice>) -> u32 {
        // 块索引
        let inner_id = inner_id as usize;

        if inner_id < INODE_DIRECT_COUNT {
            // 直接索引
            self.direct[inner_id]
        } else if inner_id < INDIRECT1_BOUND {
            // 一级索引
            get_block_cache(self.indirect1 as usize, Arc::clone(block_device))
                .lock()
                // 解析为 IndirectBlock 指向一个下一级索引块或者数据块
                .read(0, |indirect_block: &IndirectBlock| {
                    indirect_block[inner_id - INODE_DIRECT_COUNT]
                })
        } else {
            // 二级索引
            let last = inner_id - INDIRECT1_BOUND;
            // 对于二级索引的情况, 需要先查二级索引块找到挂在它下面的一级 子 索引块
            let indirect1 = get_block_cache(self.indirect2 as usize, Arc::clone(block_device))
                .lock()
                .read(0, |indirect2: &IndirectBlock| {
                    indirect2[last / INODE_INDIRECT1_COUNT]
                });
            // 再通过一级 子 索引块找到数据块
            get_block_cache(indirect1 as usize, Arc::clone(block_device))
                .lock()
                .read(0, |indirect1: &IndirectBlock| {
                    indirect1[last % INODE_INDIRECT1_COUNT]
                })
        }
    }

    // 在对文件/目录初始化之后, 它的 size 均为 0, 此时并不会索引到
    // 任何数据块, 它需要通过 increase_size 方法逐步扩充容量.
    // 在扩充的时候, 自然需要一些新的数据块来作为索引块或是保存内容的数据块.
    // 故需要先编写一些辅助方法来确定在容量扩充的时候额外需要多少块.

    /// 计算为了容纳自身 size 字节的内容需要多少个数据块
    pub fn data_blocks(&self) -> u32 {
        Self::_data_blocks(self.alloc_size)
    }

    fn _data_blocks(size: u32) -> u32 {
        // 用 size 除以每个块的大小 BLOCK_SZ 并向上取整
        (size + BLOCK_SIZE as u32 - 1) / BLOCK_SIZE as u32
    }

    pub fn total_blocks(size: u32) -> u32 {
        // total_blocks 不仅包含数据块, 还需要统计索引块

        // 调用 data_blocks 得到需要多少数据块
        let data_blocks = Self::_data_blocks(size) as usize;
        let mut total = data_blocks as usize;

        // 根据数据块数目所处的区间统计索引块

        if data_blocks > INODE_DIRECT_COUNT {
            // 一级索引
            total += 1;
        }

        if data_blocks > INDIRECT1_BOUND {
            // 二级级索引
            total += 1;

            // 二级索引的一级子索引
            total +=
                (data_blocks - INDIRECT1_BOUND - 1 + INODE_INDIRECT1_COUNT) / INODE_INDIRECT1_COUNT;
        }

        total as u32
    }

    /// 计算将一个 DiskInode 的 size 扩容到 new_size 需要额外多少个数据和索引块
    pub fn blocks_num_needed(&self, new_size: u32) -> u32 {
        assert!(new_size >= self.alloc_size);
        // 调用两次 total_blocks 作差
        Self::total_blocks(new_size) - Self::total_blocks(self.alloc_size)
    }

    /// 通过 increase_size 方法逐步扩充容量
    /// 在对文件/目录初始化之后, 它的 size 均为 0, 此时并不会索引到任何数据块.
    /// 在扩充的时候, 需要一些新的数据块来作为索引块或是保存内容的数据块.
    pub fn increase_size(
        &mut self,
        new_size: u32,
        // 保存了本次容量扩充所需块编号的向量, 这些块都是由上层的磁盘块管理器负责分配的
        new_blocks: Vec<u32>,
        block_device: &Arc<dyn BlockDevice>,
    ) {
        let mut current_blocks = self.data_blocks(); // 当前文件大小所需的数据块数目
        self.size = new_size;
        self.alloc_size = new_size;
        // Q: 为什么不用 total_block 方法
        // A: 注意参数 new_blocks 是由上层的磁盘块管理器负责分配的 (包括了索引需要使用的 block), 这里计算的 total_block 只与数据大小相关
        let mut total_blocks = self.data_blocks(); // 扩容后的总块数
        let mut new_blocks = new_blocks.into_iter();

        // 填充直接索引
        while current_blocks < total_blocks.min(INODE_DIRECT_COUNT as u32) {
            // note: index current_blocks 相当于第 current_blocks+1 块
            self.direct[current_blocks as usize] = new_blocks.next().unwrap();
            current_blocks += 1;
        }

        // 分配一级索引
        if total_blocks > INODE_DIRECT_COUNT as u32 {
            if current_blocks == INODE_DIRECT_COUNT as u32 {
                // 直接索引已经填满, 需要分配一级索引
                self.indirect1 = new_blocks.next().unwrap();
            }
            current_blocks -= INODE_DIRECT_COUNT as u32;
            total_blocks -= INODE_DIRECT_COUNT as u32;
        } else {
            return;
        }

        // 填充一级索引
        get_block_cache(self.indirect1 as usize, Arc::clone(block_device))
            .lock()
            .modify(0, |indirect1: &mut IndirectBlock| {
                while current_blocks < total_blocks.min(INODE_INDIRECT1_COUNT as u32) {
                    indirect1[current_blocks as usize] = new_blocks.next().unwrap();
                    current_blocks += 1;
                }
            });

        // 分配二级索引
        if total_blocks > INODE_INDIRECT1_COUNT as u32 {
            if current_blocks == INODE_INDIRECT1_COUNT as u32 {
                // 一级索引已经填满, 需要分配二级索引
                self.indirect2 = new_blocks.next().unwrap();
            }
            current_blocks -= INODE_INDIRECT1_COUNT as u32;
            total_blocks -= INODE_INDIRECT1_COUNT as u32;
        } else {
            return;
        }

        // 填充二级索引
        // from (a0, b0) -> (a1, b1)
        // a0 当前二级索引的索引号
        let mut a0 = current_blocks as usize / INODE_INDIRECT1_COUNT;
        // b0 当前二级索引的一级子索引的索引号
        let mut b0 = current_blocks as usize % INODE_INDIRECT1_COUNT;
        // a1 目标二级索引的索引号
        let a1 = total_blocks as usize / INODE_INDIRECT1_COUNT;
        // b1 目标二级索引的一级子索引的索引号
        let b1 = total_blocks as usize % INODE_INDIRECT1_COUNT;

        // 分配二级索引的一级子索引
        get_block_cache(self.indirect2 as usize, Arc::clone(block_device))
            .lock()
            .modify(0, |indirect2: &mut IndirectBlock| {
                while (a0 < a1) || (a0 == a1 && b0 < b1) {
                    if b0 == 0 {
                        indirect2[a0] = new_blocks.next().unwrap();
                    }

                    // 填充二级索引的一级子索引
                    get_block_cache(indirect2[a0] as usize, Arc::clone(block_device))
                        .lock()
                        .modify(0, |indirect1: &mut IndirectBlock| {
                            indirect1[b0] = new_blocks.next().unwrap();
                        });

                    // 移动到下一个一级子索引
                    b0 += 1;
                    if b0 == INODE_INDIRECT1_COUNT {
                        b0 = 0;
                        a0 += 1;
                    }
                }
            });
    }

    /// 清空文件的内容并回收所有数据和索引块
    ///
    /// 将大小清除为零并返回应释放的块, 再将块内容清零;
    /// 最后将回收的所有块的编号保存在一个向量中返回给磁盘块管理器
    pub fn clear_size(&mut self, block_device: &Arc<dyn BlockDevice>) -> Vec<u32> {
        // 保存所有需要回收的块编号
        let mut v: Vec<u32> = Vec::new();
        let mut data_blocks = self.data_blocks() as usize;
        self.size = 0;
        self.alloc_size = 0;
        // 当前已经清空的块数目 分别对应直接索引, 一级索引, 二级索引
        let mut current_blocks = 0usize;

        // 回收直接索引
        while current_blocks < data_blocks.min(INODE_DIRECT_COUNT) {
            v.push(self.direct[current_blocks]); // 保存需要回收的块编号
            self.direct[current_blocks] = 0; // 清空直接索引
            current_blocks += 1;
        }

        // 回收一级索引块
        if data_blocks > INODE_DIRECT_COUNT {
            v.push(self.indirect1);
            data_blocks -= INODE_DIRECT_COUNT;
            current_blocks = 0;
        } else {
            return v;
        }
        get_block_cache(self.indirect1 as usize, Arc::clone(block_device))
            .lock()
            .modify(0, |indirect1: &mut IndirectBlock| {
                while current_blocks < data_blocks.min(INODE_INDIRECT1_COUNT) {
                    v.push(indirect1[current_blocks]);
                    // indirect1[current_blocks] = 0; // 磁盘内容不需要清空
                    current_blocks += 1;
                }
            });
        self.indirect1 = 0; // 清空一级索引

        // 回收二级索引块
        if data_blocks > INODE_INDIRECT1_COUNT {
            v.push(self.indirect2);
            data_blocks -= INODE_INDIRECT1_COUNT;
        } else {
            return v;
        }
        assert!(data_blocks <= INODE_INDIRECT2_COUNT);
        let a1 = data_blocks / INODE_INDIRECT1_COUNT;
        let b1 = data_blocks % INODE_INDIRECT1_COUNT;
        get_block_cache(self.indirect2 as usize, Arc::clone(block_device))
            .lock()
            .modify(0, |indirect2: &mut IndirectBlock| {
                for i in 0..a1 {
                    get_block_cache(indirect2[i] as usize, Arc::clone(block_device))
                        .lock()
                        .modify(0, |indirect1: &mut IndirectBlock| {
                            // 回收二级索引块的一级子索引
                            for j in 0..INODE_INDIRECT1_COUNT {
                                v.push(indirect1[j]);
                                // indirect1[j] = 0; // 磁盘内容不需要清空
                            }
                        });
                    // 回收二级索引
                    v.push(indirect2[i]);
                    // indirect2[i] = 0; // 磁盘内容不需要清空
                }

                // 对于最后一个一级子索引块
                if b1 > 0 {
                    get_block_cache(indirect2[a1] as usize, Arc::clone(block_device))
                        .lock()
                        .modify(0, |indirect1: &mut IndirectBlock| {
                            for j in 0..b1 {
                                v.push(indirect1[j]);
                                // indirect1[j] = 0; // 磁盘内容不需要清空
                            }
                        });
                    v.push(indirect2[a1]);
                    // indirect2[a1] = 0; // 磁盘内容不需要清空
                }
            });
        self.indirect2 = 0; // 清空二级索引
        v
    }

    // 通过 DiskInode 来读写它索引的那些数据块中的数据

    /// 将文件内容从 offset 字节开始的部分读到内存中的缓冲区 buf 中, 并返回实际读到的字节数
    ///
    /// 如果文件剩下的内容还足够多, 那么缓冲区会被填满;否则文件剩下的全部内容都会被读到缓冲区中
    pub fn read_at(
        &self,
        offset: usize,
        buf: &mut [u8],
        block_device: &Arc<dyn BlockDevice>,
    ) -> usize {
        // 从 offset 开始读取内容
        let mut start = offset;
        // 取最小值
        // 如果文件剩下的内容还足够多, 那么缓冲区会被填满; 否则文件剩下的全部内容都会被读到缓冲区中
        // use size rather than alloc_size
        let end = (offset + buf.len()).min(self.size as usize);
        if start >= end {
            return 0;
        }
        // 目前是文件内部第多少个数据块
        let mut start_block = start / BLOCK_SIZE as usize;
        // 读取的字节数
        let mut read_size = 0usize;

        // 遍历位于字节区间 [start, end) 中间的那些块, 将它们视为一个 DataBlock (也就是一个字节数组),
        // 并将其中的部分内容复制到缓冲区 buf 中适当的区域

        loop {
            // 计算当前块的终止位置 (终止字节编号)
            // Q: 为什么要 +1 呢
            // A: start_block * BLOCK_SIZE != 当前块的终止位置的字节编号 (第 0 块的终止字节编号为 1 * BLOCK_SIZE)
            // Q: 为什么不在最后 -1 呢
            // A: 读取的部分为 [strat, end), 每次读取的字节数为 end_current_block - start
            let mut end_current_block = (start / BLOCK_SIZE + 1) * BLOCK_SIZE;
            end_current_block = end_current_block.min(end);

            // 读取当前块的内容

            // 要在当前块中读取的字节数量
            let block_read_size = end_current_block - start;
            // dst 作为缓冲区 buf 的一个切片, 可用于修改 buf 中的内容
            let dst = &mut buf[read_size..read_size + block_read_size];
            get_block_cache(
                // start_block 维护着目前是文件内部第多少个数据块,
                // 需要首先调用 get_block_id 从索引中查到这个数据块在块设备中的块编号,
                // 随后才能传入 get_block_cache 中将正确的数据块缓存到内存中进行访问
                self.get_block_id(start_block as u32, block_device) as usize,
                Arc::clone(block_device),
            )
            .lock()
            .read(0, |data_blocks: &DataBlock| {
                let src = &data_blocks[start % BLOCK_SIZE..start % BLOCK_SIZE + block_read_size];
                dst.copy_from_slice(src);
            });

            read_size += block_read_size;

            if end_current_block == end {
                break;
            }
            // 转到下一个块
            start_block += 1;
            start = end_current_block;
        }
        read_size
    }

    /// 将数据写入当前磁盘 inode
    /// 只要 Inode 管理的数据块的大小足够, 传入的整个缓冲区的数据都必定会被写入到文件中.
    /// 注意, 当从 offset 开始的区间超出了文件范围的时候, 需要调用者在调用 write_at 之前提前调用 increase_size.
    pub fn write_at(
        &mut self,
        offset: usize,
        buf: &[u8],
        block_device: &Arc<dyn BlockDevice>,
    ) -> usize {
        // 从 offset 开始读取内容
        let mut start = offset;
        // 取最小值
        // use alloc_size rather than size
        let end = (offset + buf.len()).min(self.alloc_size as usize);
        assert!(start <= end);
        // 目前是文件内部第多少个数据块
        let mut start_block = start / BLOCK_SIZE as usize;
        let mut write_size = 0usize;

        loop {
            // 计算当前块的终止位置
            let mut end_current_block = (start / BLOCK_SIZE + 1) * BLOCK_SIZE;
            end_current_block = end_current_block.min(end);
            let block_write_size = end_current_block - start;

            get_block_cache(
                // start_block 维护着目前是文件内部第多少个数据块,
                // 需要首先调用 get_block_id 从索引中查到这个数据块在块设备中的块编号,
                // 随后才能传入 get_block_cache 中将正确的数据块缓存到内存中进行访问
                self.get_block_id(start_block as u32, block_device) as usize,
                Arc::clone(block_device),
            )
            .lock()
            .modify(0, |data_blocks: &mut DataBlock| {
                let src = &buf[write_size..write_size + block_write_size];
                let dst =
                    &mut data_blocks[start % BLOCK_SIZE..start % BLOCK_SIZE + block_write_size];
                dst.copy_from_slice(src);
            });

            write_size += block_write_size;

            if end_current_block == end {
                break;
            }
            // 转到下一个块
            start_block += 1;
            start = end_current_block;
        }

        // BUG(for disk_inode.size): 使用 chname 会导致文件大小不正确
        // 当时写这条语句 (self.size = end as u32) 的目的是为了 写文件时 不让读取后面的内容:
        // 从 offset 开始写入 content, 只覆盖content的长度, 但我的展示方式是不让看后面的部分
        //
        // fix: 不应该在这个地方更新文件大小 (self.size = end as u32)
        //      区分情况: 文件 与 文件夹
        //      请在全局使用 write 的地方注意对 size 的更新
        //      区分文件与文件夹的原因是目前并没有实现对删除目录下的目录项后回收的操作
        //
        // 解释: 对于目录项本身的回收比较简单: inode.clear() 但是, 对于父亲(目录)来说比较麻烦;
        //      删除目录项后, 对于目录项的父亲 (目录), 它的其他目录项仍然可能占用磁盘块, 没法使用 data_dealloc 回收
        //
        // 另外, 在 write 之前会调用 increase_size 不必担心 size 不对
        // self.size = end as u32; // 更新文件大小
        write_size
    }
}

// 作为一个文件而言, 它的内容在文件系统看来没有任何既定的格式, 都只是一个字节序列
// 因此每个保存内容的数据块都只是一个字节数组. 然而, 目录的内容却需要遵从一种特殊的格式.
// 在我们的实现中, 它可以看成一个目录项的序列, 每个目录项都是一个二元组,
// 二元组的首个元素是目录下面的一个文件 (或子目录) 的文件名 (或目录名),
// 另一个元素则是文件(或子目录)所在的索引节点编号.
// 目录项相当于目录树结构上的子树节点, 我们需要通过它来一级一级的找到实际要访问的文件或目录
#[repr(C)]
/// 目录项
///
/// 它自身占据空间 32 字节, 每个数据块可以存储 16 个目录项
pub struct DirEntry {
    /// 目录项 Dirent 最大允许保存长度为 27 的文件/目录名 (数组 name 中最末的一个字节留给 '\0')
    name: [u8; NAME_LENGTH_LIMIT + 1], // 28B
    inode_id: u32, // 4B
}

impl DirEntry {
    /// 创建一个空的目录项
    pub fn create_empty() -> Self {
        Self {
            name: [0; NAME_LENGTH_LIMIT + 1],
            inode_id: 0,
        }
    }

    /// 通过文件名和 inode 编号创建一个目录项
    pub fn new(name: &str, inode_id: u32) -> Self {
        let mut name_bytes = [0; NAME_LENGTH_LIMIT + 1];
        name_bytes[..name.len()].copy_from_slice(name.as_bytes());
        Self {
            name: name_bytes,
            inode_id,
        }
    }

    // 在从目录的内容中读取目录项或者是将目录项写入目录的时候,
    // 我们需要将目录项转化为缓冲区(即字节切片)的形式来符合索引节点 Inode 数据结构中的 read_at 或 write_at 方法接口的要求

    /// 序列化目录项
    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(self as *const Self as usize as *const u8, DIRENT_SIZE)
        }
    }

    /// 序列化目录项
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        unsafe {
            std::slice::from_raw_parts_mut(self as *mut Self as usize as *mut u8, DIRENT_SIZE)
        }
    }

    pub fn name(&self) -> &str {
        let len = (0usize..).find(|&i| self.name[i] == 0).unwrap(); // 找到第一个 0
        std::str::from_utf8(&self.name[..len]).unwrap()
    }

    pub fn chname(&mut self, name: &str) {
        self.name[..name.len()].copy_from_slice(name.as_bytes());
        self.name[name.len()] = 0;
    }

    pub fn inode_id(&self) -> u32 {
        self.inode_id
    }
}
