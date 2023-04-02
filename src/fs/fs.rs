//! 各整合段区域及磁盘数据结构结构, 实现 fs 的整体磁盘布局
//!
//! [`FileSystem`] 知道每个布局区域所在的位置, 磁盘块的分配和回收也需要经过它才能完成, 因此某种意义上讲它可以看成一个磁盘块管理器
//!
//! 从这一层开始, 所有的数据结构放在内存上

use std::sync::Arc;

use spin::Mutex;

use super::{
    block_cache_sync_all, get_block_cache, Bitmap, BlockDevice, DiskInode, DiskInodeType, Inode,
    SuperBlock, BLOCK_SIZE,
};

/// 文件系统 (磁盘块管理器)
///
/// Blocks: Super Block(0) -> Inode Bit Map Blocks -> Inode Blocks -> Data Bit Map Blocks -> Data Blocks
pub struct FileSystem {
    /// 保留块设备的一个指针 block_device,
    /// 在进行后续操作的时候, 该指针会被拷贝并传递给下层的数据结构,
    /// 让它们也能够直接访问块设备.
    pub block_device: Arc<dyn BlockDevice>,
    /// 索引节点位图
    /// 一位代表一个索引节点, 一个块中存放4个索引节点
    pub inode_bitmap: Bitmap,
    /// 数据块位图
    /// 一位代表一个数据块
    pub data_bitmap: Bitmap,
    /// 索引区域起始块号
    inode_area_start_block: u32,
    /// 数据区域起始块号
    data_area_start_block: u32,
}

type DataBlock = [u8; BLOCK_SIZE];

impl FileSystem {
    /// 在块设备上创建并初始化一个文件系统
    pub fn create(
        block_device: Arc<dyn BlockDevice>,
        total_blocks: u32,        // 磁盘总块数
        inode_bitmap_blocks: u32, // 索引节点位图占用的块数
    ) -> Arc<Mutex<Self>> {
        // 根据传入的参数计算每个区域各应该包含多少块

        let inode_bitmap = Bitmap::new(
            // note: inode 位图的起始块号是 1 (0 是超级块)
            1,
            inode_bitmap_blocks as usize,
        );

        // 根据 inode 位图的大小计算 inode 区域至少需要多少个块,
        // 使 inode 位图中的每个bit都能够有一个实际的 inode 可以对应,
        // 确定 inode 位图区域和 inode 区域的大小

        // 计算 inode 数量
        // 根据 inode_bitmap_blocks (占用的磁盘块数) 计算出 inode 数量
        let inode_num = inode_bitmap.maximum();

        // inode 区域大小
        let inode_area_blocks =
            // 向上取整
            ((inode_num * std::mem::size_of::<DiskInode>() + BLOCK_SIZE - 1) / BLOCK_SIZE) as u32;

        // 索引节点使用总的块数 等于 索引节点位图占用的块数 加上 索引节点区域占用的块数
        let inode_total_blocks = inode_area_blocks + inode_bitmap_blocks;

        // 剩下的块都分配给 数据块位图区域 和 数据块区域

        // 总的数据块数 等于 磁盘总块数 减去 索引节点总的块数
        // Q: 为什么再减去 1 呢?(减去的 1 是超级块, block_id = 0)
        let data_total_blocks = total_blocks - 1 - inode_total_blocks;

        // 数据块位图区域大小
        //
        // Q: 为什么要除以 4097 呢? 为什么不是除以 4096 呢?
        //
        // 我们希望位图覆盖后面的数据块的前提下数据块尽量多.
        // 但要求数据块位图中的每个 bit 仍然能够对应到一个数据块,
        // 数据块位图又不能过小, 不然会造成某些数据块永远不会被使用.
        // 设数据的位图占据 x 个块, 则该位图能管理的数据块不超过 4096 * x.
        // 数据区域总共 data_total_blocks 个块, 除了数据位图的块剩下都是数据块,
        // 也就是位图管理的数据块为 data_total_blocks - x 个块.
        // 于是有不等式 data_total_blocks - x <= 4096 * x,
        // 得到 x >= data_total_blocks / 4097.
        // 数据块尽量多也就要求位图块数尽量少, 于是取 x 的最小整数解也就是 data_total_blocks / 4097 上取整, 也就是代码中的表达式.
        // 因此数据块位图区域最合理的大小是剩余的块数除以 4097 再上取整.
        //
        let data_bitmap_blocks = (data_total_blocks + 4096) / 4097;

        // 数据块区域大小
        let data_area_blocks = data_total_blocks - data_bitmap_blocks;

        // 初始化数据块位图
        let data_bitmap = Bitmap::new(
            // inode_bitmap_blocks + inode_area_blocks = inode_total_blocks; + 1 is the super block
            (1 + inode_bitmap_blocks + inode_area_blocks) as usize,
            data_bitmap_blocks as usize,
        );

        // 初始化文件系统
        let mut fs = Self {
            block_device: Arc::clone(&block_device),
            inode_bitmap,
            data_bitmap,
            // Q: 为什么不是从 0 开始计算的: 0 这个块存放了其他信息(超级块)
            // 在 inode_area 之前存放了 inode_bitmap, 故 inode_area 的起始块号为 inode_bitmap_blocks + 1
            inode_area_start_block: 1 + inode_bitmap_blocks,
            // 在 data_area 之前存放了 inode_bitmap, inode_area, data_bitmap, 故 data_area 的起始块号为 inode_bitmap_blocks + inode_area_blocks + 2
            data_area_start_block: 1 + inode_total_blocks + data_bitmap_blocks,
        };

        // 既然是创建文件系统, 第一次使用, 需要将块设备的前 total_blocks 个块清零
        for i in 0..total_blocks {
            get_block_cache(i as usize, Arc::clone(&block_device))
                .lock()
                .modify(0, |data_block: &mut DataBlock| {
                    // 以块为单位, 将块中的所有字节都设置为 0
                    for byte in data_block.iter_mut() {
                        *byte = 0;
                    }
                });
        }

        // 初始化超级块
        // 将位于块设备编号为 0 块上的超级块进行初始化, 只需传入之前计算得到的每个区域的块数就行
        get_block_cache(0, Arc::clone(&block_device)).lock().modify(
            0,
            |super_block: &mut SuperBlock| {
                super_block.initialize(
                    total_blocks,
                    inode_bitmap_blocks,
                    inode_area_blocks,
                    data_bitmap_blocks,
                    data_area_blocks,
                );
            },
        );

        // 为根目录 "/" 创建一个 inode
        // 首先需要调用 alloc_inode 在 inode 位图中分配一个 inode ,
        // 由于这是第一次分配, 它的编号固定是 0 .
        assert_eq!(fs.alloc_inode(), 0);

        // 将分配到的 inode 初始化为 fs 中的根目录,
        // 故需要调用 get_disk_inode_pos 来根据 inode 编号获取该 inode 所在的块的编号以及块内偏移,
        // 之后就可以将它们传给 get_block_cache 和 modify 了
        let (root_inode_block_id, root_inode_offset) = fs.get_disk_inode_pos(0);

        get_block_cache(root_inode_block_id as usize, Arc::clone(&block_device))
            .lock()
            .modify(root_inode_offset, |disk_inode: &mut DiskInode| {
                disk_inode.initialize(DiskInodeType::Directory);
            });

        block_cache_sync_all();

        Arc::new(Mutex::new(fs))
    }

    /// 通过 inode_id
    /// 返回 block_id 和 offset
    //
    // Q: 那么删除是不是可以解决
    pub fn get_disk_inode_pos(&self, inode_id: u32) -> (u32, usize) {
        let inode_size = std::mem::size_of::<DiskInode>();
        // 每块有多少 inode
        // inodes_per_block = BLOCK_SIZE / inode_size = 512 / 128 = 4,  表示每个块中有 4 个 inode
        let inodes_pre_block = (BLOCK_SIZE / inode_size) as u32;
        let block_id = self.inode_area_start_block + inode_id / inodes_pre_block;
        (
            block_id,
            (inode_id % inodes_pre_block) as usize * inode_size,
        )
    }

    /// 获取 数据块 通过 id
    #[allow(unused)]
    pub fn get_data_block_id(&self, data_block_id: u32) -> u32 {
        self.data_area_start_block + data_block_id
    }

    // alloc_data 和 dealloc_data 分配/回收数据块传入/返回的参数都表示数据块在块设备上的编号, 而不是在数据块位图中分配的bit编号

    /// 分配索引
    ///
    /// 首先需要获取 inode_bitmap 所在的磁盘块,
    /// 以 bit 组(每组 64 bits)为单位进行遍历,
    /// 找到一个尚未被全部分配出去的组,
    /// 最后在里面分配一个 bit.
    pub fn alloc_inode(&mut self) -> u32 {
        self.inode_bitmap.alloc(&self.block_device).unwrap() as u32
    }

    /// 分配数据块
    pub fn alloc_data(&mut self) -> u32 {
        self.data_bitmap.alloc(&self.block_device).unwrap() as u32 + self.data_area_start_block
    }

    /// 回收数据块
    pub fn dealloc_data(&mut self, block_id: u32) {
        get_block_cache(block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(0, |data_block: &mut DataBlock| {
                data_block.iter_mut().for_each(|p| {
                    *p = 0;
                })
            });
        self.data_bitmap.dealloc(
            &self.block_device,
            (block_id - self.data_area_start_block) as usize,
        )
    }

    // maybe
    #[allow(unused)]
    pub fn dealloc_inode(&mut self, inode_id: u32) {
        // 由于一个块中可以存放 4 个索引节点, 因此相较于删除数据节点,
        // inode_id 对应的数据大小为 DirEntry 的大小, 也就是 128 字节
        // 而 block_id 对应的数据大小为 DataBlock 的大小, 也就是 512 字节
        // 删除索引节点没那么容易 (可能需要修改数据结构)
        // 不可以直接这样对块内的数据进行清零
        // get_block_cache(inode_id as usize, Arc::clone(&self.block_device)) // 参数不应该是 inode_id
        //     .lock()
        //     .modify(0, |data_block: &mut DataBlock| {
        //         data_block.iter_mut().for_each(|p| {
        //             *p = 0;
        //         })
        //     });
        self.inode_bitmap.dealloc(
            &self.block_device,
            (inode_id - self.inode_area_start_block) as usize,
        )
    }

    // 通过 open 方法可以从一个已写入了 fs 镜像的块设备上打开 fs
    pub fn open(block_device: Arc<dyn BlockDevice>) -> Arc<Mutex<Self>> {
        // 读超级块: 超级块的索引 id 为 0
        get_block_cache(0, Arc::clone(&block_device))
            .lock()
            .read(0, |super_block: &SuperBlock| {
                assert!(super_block.is_valid(), "Error loading EFS!");

                let inode_total_blocks =
                    super_block.inode_bitmap_blocks + super_block.inode_area_blocks;

                let fs = Self {
                    block_device,
                    inode_bitmap: Bitmap::new(1, super_block.inode_bitmap_blocks as usize),
                    data_bitmap: Bitmap::new(
                        (1 + inode_total_blocks) as usize,
                        super_block.data_bitmap_blocks as usize,
                    ),
                    inode_area_start_block: 1 + super_block.inode_bitmap_blocks,
                    // FIX: BUG for dealloc_data
                    data_area_start_block: 1 + inode_total_blocks + super_block.data_bitmap_blocks,
                };

                Arc::new(Mutex::new(fs))
            })
    }

    // 文件系统的使用者在通过 FileSystem::open 从装载了 fs 镜像的块设备上打开 efs 之后,
    // 要做的第一件事情就是获取根目录的 Inode .
    //
    // 因为 FileSystem 目前仅支持绝对路径, 对于任何文件/目录的索引都必须从根目录开始向下逐级进行.
    // 等到索引完成之后,  FileSystem 才能对文件/目录进行操作.
    //
    // 事实上 FileSystem 提供了另一个名为 root_inode 的方法来获取根目录的 Inode

    /// 获取文件系统的根inode
    pub fn root_inode(fs: &Arc<Mutex<Self>>) -> Inode {
        // acquire fs lock temporarily
        let block_device = Arc::clone(&fs.lock().block_device);
        // release fs lock (lock used in stack 在函数中使用, 释放 drop 掉)

        // acquire fs lock temporarily
        // 对于 root_inode 的初始化, 是在调用 Inode::new 时将传入的 inode_id 设置为 0 ,
        // 因为根目录对应于文件系统中第一个分配的 inode , 因此它的 inode_id 总会是 0 .
        let (block_id, block_offset) = fs.lock().get_disk_inode_pos(0);
        // release fs lock

        // 不会在调用 Inode::new 过程中尝试获取整个 FileSystem 的锁来查询 inode 在块设备中的位置,
        // 而是在调用它之前预先查询并作为参数传过去
        Inode::new(block_id, block_offset, Arc::clone(fs), block_device)
    }

    // TODO: dealloc_inode
}
