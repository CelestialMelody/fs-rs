//! 块缓存层: 在内存中缓存磁盘块的数据, 避免频繁读写磁盘
//! 由于操作系统频繁读写速度缓慢的磁盘块会极大降低系统性能,
//! 因此常见的手段是先通过 read_block 将一个块上的数据从磁盘读到内存中的一个缓冲区 [`BlockCache`] 中,
//! 这个缓冲区中的内容是可以直接读写的, 那么后续对这个数据块的大部分访问就可以在内存中完成了.
//! 如果缓冲区中的内容被修改了, 那么后续还需要通过 write_block 将缓冲区中的内容写回到磁盘块中.
//!
//! 无论站在代码实现鲁棒性还是性能的角度, 将这些缓冲区合理的管理起来都是很有必要的.
//! 一种完全不进行任何管理的模式可能是:
//! 每当要对一个磁盘块进行读写的时候, 都通过 read_block 将块数据读取到一个 临时 创建的缓冲区, 并在进行一些操作之后(可选地)将缓冲区的内容写回到磁盘块.
//! 从性能上考虑, 我们需要尽可能降低实际块读写(即 read/write_block )的次数, 因为每一次调用它们都会产生大量开销.
//! 要做到这一点, 关键就在于对块读写操作进行 合并 .
//! 例如, 如果一个块已经被读到缓冲区中了, 那么我们就没有必要再读一遍, 直接用已有的缓冲区就行了;
//! 同时, 对于缓冲区中的同一个块的多次修改没有必要每次都写回磁盘, 只需等所有的修改都结束之后统一写回磁盘即可.
//!
//! 当磁盘上的数据结构比较复杂的时候, 很难通过应用来合理地规划块读取/写入的时机.
//! 这不仅可能涉及到复杂的参数传递, 稍有不慎还有可能引入同步性问题(目前可以暂时忽略):
//! 即一个块缓冲区修改后的内容在后续的同一个块读操作中不可见, 这很致命但又难以调试.
//!
//! 因此, 我们的做法是将缓冲区统一管理起来. [`BlockCacheManager`]
//! 当我们要读写一个块的时候, 首先就是去全局管理器中查看这个块是否已被缓存到内存缓冲区中.
//! 如果是这样, 则在一段连续时间内对于一个块进行的所有操作均是在同一个固定的缓冲区中进行的, 这解决了同步性问题.
//! 此外, 通过 read/write_block 进行块实际读写的时机完全交给块缓存层的全局管理器处理, 上层子系统无需操心.
//! 全局管理器会尽可能将更多的块操作合并起来, 并在必要的时机发起真正的块实际读写.

use std::{
    collections::VecDeque,
    // sync::{Arc, Mutex},
    sync::Arc,
};

use lazy_static::*;
use spin::Mutex; // https://docs.rs/spin/0.5.2/spin/struct.Mutex.html

use super::{BlockDevice, BLOCK_CACHE_SIZE, BLOCK_SIZE};

/// Cached block inside memory
pub struct BlockCache {
    /// cache 是一个 512 字节的数组(恰好为一个块), 表示位于内存中的缓冲区
    cache: [u8; BLOCK_SIZE],
    /// block_id 记录了这个块缓存来自于磁盘中的块的编号
    block_id: usize,
    /// block_device 是一个底层块设备的引用, 可通过它进行块读写
    block_device: Arc<dyn BlockDevice>,
    /// modified 记录这个块从磁盘载入内存缓存之后, 它有没有被修改过
    modified: bool,
}

impl BlockCache {
    /// 创建一个 BlockCache: 这将触发一次 read_block 将一个块上的数据从磁盘读到缓冲区 cache
    pub fn new(block_id: usize, block_device: Arc<dyn BlockDevice>) -> Self {
        let mut cache = [0u8; BLOCK_SIZE];
        block_device.read_block(block_id, &mut cache);
        Self {
            cache,
            block_id,
            block_device,
            modified: false,
        }
    }

    /// 得到一个 BlockCache 内部的缓冲区中指定偏移量 offset 的字节地址
    fn addr_of_offset(&self, offset: usize) -> usize {
        &self.cache[offset] as *const u8 as usize
    }

    /// 获取缓冲区中的位于偏移量 offset 的一个类型为 T 的磁盘上数据结构的不可变引用.
    /// 该泛型方法的 Trait Bound 限制类型 T 必须是一个编译时已知大小的类型;
    /// 这里编译器会自动进行生命周期标注, 约束返回的引用的生命周期不超过 BlockCache 自身, 在使用的时候我们会保证这一点.
    pub fn get_ref<T>(&self, offset: usize) -> &T
    where
        T: Sized,
    {
        let type_size = std::mem::size_of::<T>();
        // 确认 T 被整个包含在磁盘块及其缓冲区之内
        assert!(offset + type_size <= BLOCK_SIZE);
        let addr = self.addr_of_offset(offset);
        // &* 再借用; 将指针转换为引用
        unsafe { &*(addr as *const T) }
    }

    /// get_mut 会获取磁盘上数据结构的可变引用, 由此可以对数据结构进行修改.
    /// 由于这些数据结构目前位于内存中的缓冲区中,
    /// 我们需要将 BlockCache 的 modified 标记为 true 表示该缓冲区已经被修改,
    /// 之后需要将数据写回磁盘块才能真正将修改同步到磁盘.
    pub fn get_mut<T>(&mut self, offset: usize) -> &mut T
    where
        T: Sized,
    {
        let type_size = std::mem::size_of::<T>();
        assert!(offset + type_size <= BLOCK_SIZE);
        self.modified = true;
        let addr = self.addr_of_offset(offset);
        unsafe { &mut *(addr as *mut T) }
    }

    // 思考: 为什么使用闭包来实现对缓冲区的读写操作
    //
    // 将 get_ref/get_mut 进一步封装为更为易用的形式.
    // 在 BlockCache 缓冲区偏移量为 offset 的位置获取一个类型为 T 的磁盘上数据结构的不可变/可变引用(分别对应 read/modify ),
    // 并让它执行传入的闭包 f 中所定义的操作.
    //
    // 注意 read/modify 的返回值是和传入闭包的返回值相同的,
    // 因此相当于 read/modify 构成了传入闭包 f 的一层执行环境, 让它能够绑定到一个缓冲区上执行.
    //
    // 这里我们传入闭包的类型为 FnOnce , 这是因为闭包里面的变量被捕获的方式涵盖了不可变引用/可变引用/和 move 三种可能性,
    // 故而我们需要选取范围最广的 FnOnce .
    //
    // 参数中的 impl 关键字体现了一种类似泛型的静态分发功能.

    pub fn read<T, V>(&self, offset: usize, f: impl FnOnce(&T) -> V) -> V {
        f(self.get_ref(offset))
    }

    pub fn modify<T, V>(&mut self, offset: usize, f: impl FnOnce(&mut T) -> V) -> V {
        f(self.get_mut(offset))
    }

    /// If modified, write back to disk when dropped.
    ///
    /// 事实上,  sync 并不是只有在 drop 的时候才会被调用.
    /// 在 Linux 中, 通常有一个后台进程负责定期将内存中缓冲区的内容写回磁盘.
    /// 另外有一个 sys_fsync 系统调用可以让应用主动通知内核将一个文件的修改同步回磁盘.
    /// 由于我们的实现比较简单,  sync 仅会在 BlockCache 被 drop 时才会被调用.
    pub fn sync(&mut self) {
        if self.modified {
            self.block_device.write_block(self.block_id, &self.cache);
            self.modified = false;
        }
    }
}

impl Drop for BlockCache {
    /// BlockCache 的设计体现了 RAII 思想,  它管理着一个缓冲区的生命周期.
    /// 当 BlockCache 的生命周期结束之后缓冲区也会被从内存中回收,
    /// 这个时候 modified 标记将会决定数据是否需要写回磁盘.
    /// 在 BlockCache 被 drop 的时候, 它会首先调用 sync 方法,
    /// 如果自身确实被修改过的话才会将缓冲区的内容写回磁盘.
    fn drop(&mut self) {
        self.sync();
    }
}

/// 块缓存全局管理器的功能是:
///
/// 当我们要对一个磁盘块进行读写时, 首先看它是否已经被载入到内存缓存中了,
/// 如果已经被载入的话则直接返回, 否则需要先读取磁盘块的数据到内存缓存中.
///
/// 此时, 如果内存中驻留的磁盘块缓冲区的数量已满,
/// 则需要遵循某种缓存替换算法将某个块的缓存从内存中移除,
/// 再将刚刚读到的块数据加入到内存缓存中.
///
/// 我们这里使用一种类 FIFO 的简单缓存替换算法, 因此在管理器中只需维护一个队列
pub struct BlockCacheManager {
    // 使用 Arc<T> 包装一个 Mutex<T> 能够实现在多线程之间共享所有权
    //
    // Rust Pattern卡片:  Arc<Mutex<?>>
    //
    // 先看下 Arc 和 Mutex 的正确配合可以达到支持多线程安全读写数据对象.
    // 如果需要多线程共享所有权的数据对象, 则只用 Arc 即可.
    // 如果需要修改 T 类型中某些成员变量 member, 那直接采用 Arc<Mutex<T>> ,
    // 并在修改的时候通过 obj.lock().unwrap().member = xxx 的方式是可行的,
    // 但这种编程模式的同步互斥的粒度太大, 可能对互斥性能的影响比较大.
    // 为了减少互斥性能开销, 其实只需要在 T 类型中的 需要被修改的成员变量上 加 Mutex<_> 即可.
    // 如果成员变量也是一个数据结构, 还包含更深层次的成员变量, 那应该继续下推到最终需要修改的成员变量上去添加 Mutex .
    //
    /// 队列 queue 中管理的是块编号和块缓存的二元组.
    /// 块编号的类型为 usize , 而块缓存的类型则是一个 Arc<Mutex<BlockCache>> .
    ///
    /// 这是一个此前频频提及到的 Rust 中的经典组合, 它可以同时提供共享引用和互斥访问.
    /// 这里的共享引用意义在于块缓存既需要在管理器 BlockCacheManager 保留一个引用,
    /// 还需要以引用的形式返回给块缓存的请求者让它可以对块缓存进行访问.
    /// 而互斥访问在单核上的意义在于提供内部可变性通过编译, 在多核环境下则可以帮助我们避免可能的并发冲突.
    ///
    /// 事实上, 一般情况下我们需要在更上层提供保护措施避免两个线程同时对一个块缓存进行读写,
    /// 因此这里只是比较谨慎的留下一层保险.
    /// 注意:  VecDeque 中只以 block_id 作为标识的话, 同时读写不同设备的同一个 block 时会有冲突
    queue: VecDeque<(usize, Arc<Mutex<BlockCache>>)>,
}

/**
    // 修改 queue 为Vec
    pub struct BlockCacheManager {
        queue: Vec<(usize, Arc<Mutex<BlockCache>>)>,
    }

    impl BlockCacheManager {
        pub fn get_block_cache(
            &mut self,
            block_id: usize,
            block_device: Arc<dyn BlockDevice>,
        ) -> Arc<Mutex<BlockCache>> {
            if let Some((_, cache)) = self.queue.iter().find(|(id, _)| *id == block_id) {
                cache.clone()
            } else {
                // substitute
                if self.queue.len() == BLOCK_CACHE_SIZE {
                    // from front to tail
                    if let Some((idx, _)) = self
                        .queue
                        .iter()
                        .enumerate()
                        .find(|(_, (_, cache))| Arc::strong_count(cache) == 1)
                    {
                        self.queue.swap_remove(idx);
                    } else {
                        panic!("Run out of BlockCache!");
                    }
                }
                // load block into mem and push back
                let block_cache = Arc::new(Mutex::new(BlockCache::new(block_id, block_device.clone())));
                self.queue.push((block_id, Arc::clone(&block_cache)));
                block_cache
            }
        }
    }
*/

impl BlockCacheManager {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }

    /// 尝试从块缓存管理器中获取一个编号为 block_id 的块的块缓存,
    /// 如果找不到, 会从磁盘读取到内存中, 还有可能会发生缓存替换
    pub fn get_block_cache(
        &mut self,
        block_id: usize,
        block_device: Arc<dyn BlockDevice>,
    ) -> Arc<Mutex<BlockCache>> {
        // 遍历整个队列试图找到一个编号相同的块缓存,
        // 如果找到了, 会将块缓存管理器中保存的块缓存的引用复制一份并返回
        if let Some(pair) = self.queue.iter().find(|pair| pair.0 == block_id) {
            Arc::clone(&pair.1)
        } else {
            // 如果找不到, 此时必须将块从磁盘读入内存中的缓冲区.
            // 在实际读取之前, 需要判断管理器保存的块缓存数量是否已经达到了上限.
            // 如果达到了上限, 需要执行缓存替换算法, 丢掉某个块缓存并空出一个空位.
            if self.queue.len() == BLOCK_CACHE_SIZE {
                // 这里使用一种类 FIFO 算法:
                // 每加入一个块缓存时要从队尾加入, 要替换时则从队头弹出.
                if let Some((idx, _)) = self
                    .queue
                    .iter()
                    .enumerate()
                    // 但此时队头对应的块缓存可能仍在使用:
                    // 判断的标志是其强引用计数, 即除了块缓存管理器保留的一份副本之外, 在外面还有若干份副本正在使用.
                    // 因此, 我们的做法是从队头遍历到队尾找到第一个强引用计数恰好为 1 的块缓存并将其替换出去.
                    .find(|(_, pair)| Arc::strong_count(&pair.1) == 1)
                {
                    self.queue.drain(idx..=idx); // 从队列中删除该块缓存, range: [idx, idx] == idx
                } else {
                    // 那么是否有可能出现队列已满且其中所有的块缓存都正在使用的情形呢?
                    // 事实上, 只要我们的上限 BLOCK_CACHE_SIZE 设置的足够大, 超过所有应用同时访问的块总数上限, 那么这种情况永远不会发生.
                    // 但是, 如果我们的上限设置不足, 内核将 panic (基于简单内核设计的思路).
                    panic!("Run out of BlockCache");
                }
            }
            // 创建一个新的块缓存(会触发 read_block 进行块读取)并加入到队尾, 最后返回给请求者.
            let block_cache = Arc::new(Mutex::new(BlockCache::new(
                block_id,
                Arc::clone(&block_device),
            )));
            self.queue.push_back((block_id, Arc::clone(&block_cache)));
            block_cache
        }
    }
}

lazy_static! {
    pub static ref BLOCK_CACHE_MANAGER: Mutex<BlockCacheManager> =
        Mutex::new(BlockCacheManager::new());
}

/// 尝试从块缓存管理器中获取一个编号为 block_id 的块的块缓存,
/// 如果找不到, 会从磁盘读取到内存中, 还有可能会发生缓存替换
///
/// 对于其他模块而言, 可以直接通过 get_block_cache 方法来请求块缓存.
///
/// 它返回的是一个 Arc<Mutex<BlockCache>>,
/// 调用者需要通过 .lock() 获取里层互斥锁 Mutex 才能对最里面的 BlockCache 进行操作,
/// 比如通过 read/modify 访问缓冲区里面的磁盘数据结构.
pub fn get_block_cache(
    block_id: usize,
    block_device: Arc<dyn BlockDevice>,
) -> Arc<Mutex<BlockCache>> {
    BLOCK_CACHE_MANAGER
        .lock() // use spin lock: https://docs.rs/spin/0.5.2/spin/struct.Mutex.html
        // .unwrap() // use std
        .get_block_cache(block_id, block_device)
}

pub fn block_cache_sync_all() {
    let manager = BLOCK_CACHE_MANAGER.lock();
    for (_, block_cache) in manager.queue.iter() {
        block_cache.lock().sync();
    }
}
