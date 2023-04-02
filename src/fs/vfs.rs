//! FileSystem 实现了磁盘布局并能够将磁盘块有效的管理起来.
//! 但是对于文件系统的使用者而言, 他们往往不关心磁盘布局是如何实现的, 而是更希望能够直接看到目录树结构中逻辑上的文件和目录.
//! 为此需要设计索引节点 [`Inode`] 暴露给文件系统的使用者, 让他们能够直接对文件和目录进行操作.
//!
//!  DiskInode 放在磁盘块中比较固定的位置, 而 Inode 是放在内存中的记录文件索引节点信息的数据结构

use std::sync::Arc;

use crate::fs::{DirEntry, DIRENT_SIZE};

use ::log::error;

use super::{
    block_cache_sync_all, fs::FileSystem, get_block_cache, BlockDevice, DiskInode, DiskInodeType,
};

use spin::{Mutex, MutexGuard};

pub struct Inode {
    /// 位于哪个盘块(Inode位于的磁盘块)
    block_id: usize,
    /// 盘块上的偏移
    block_offset: usize,
    fs: Arc<Mutex<FileSystem>>,
    block_device: Arc<dyn BlockDevice>,
}

impl Inode {
    pub fn new(
        block_id: u32,
        block_offset: usize,
        fs: Arc<Mutex<FileSystem>>,
        block_device: Arc<dyn BlockDevice>,
    ) -> Self {
        Self {
            block_id: block_id as usize,
            block_offset,
            fs,
            block_device,
        }
    }

    // 仿照 BlockCache::read/modify ,
    // 我们可以设计两个方法来简化对于 Inode 对应的磁盘上的 DiskInode 的访问流程,
    // 而不是每次都需要 get_block_cache.lock.read/modify

    /// 在磁盘 inode 上调用一个函数来读取它
    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .read(self.block_offset, f)
    }

    /// 在磁盘 inode 上调用一个函数来修改它
    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .modify(self.block_offset, f)
    }

    // 文件索引
    // USED:
    // 在目录树上仅有一个目录--那就是作为根节点的根目录. 所有的文件都在根目录下面.
    // 于是, 我们不必实现目录索引.
    // 文件索引的查找比较简单, 仅需在根目录的目录项中根据文件名找到文件的 inode 编号即可.
    // 由于没有子目录的存在, 这个过程只会进行一次

    // FEAT: 现在支持目录了

    /// 根据名称查找磁盘 inode 下的 inode
    fn find_inode_id(&self, name: &str, disk_inode: &DiskInode) -> Option<u32> {
        assert!(disk_inode.is_dir()); // 一定是目录
        let file_count = (disk_inode.size as usize) / DIRENT_SIZE;
        let mut dir_entry = DirEntry::create_empty();
        for i in 0..file_count {
            assert_eq!(
                disk_inode.read_at(
                    DIRENT_SIZE * i,
                    dir_entry.as_bytes_mut(),
                    &self.block_device,
                ),
                DIRENT_SIZE,
            ); // 读取目录项

            // 将目录内容中的所有目录项都读到内存进行逐个比对
            // 如果能够找到, 则 find 方法会根据查到 inode 编号, 对应生成一个 Inode 用于后续对文件的访问
            if dir_entry.name() == name {
                return Some(dir_entry.inode_id() as u32);
            }
        }
        None
    }

    pub fn find(&self, name: &str) -> Option<Arc<Inode>> {
        let fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            // 通过偏移 获取一个 disk_inode; 通过 get_ref(offset) 获取
            // 它首先调用 find_inode_id 方法
            self.find_inode_id(name, disk_inode).map(|inode_id| {
                let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
                Arc::new(Self::new(
                    block_id,
                    block_offset,
                    self.fs.clone(),
                    self.block_device.clone(),
                ))
            })
        })
    }

    pub fn is_dir(&self) -> bool {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| disk_inode.is_dir())
    }

    pub fn size(&self) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| disk_inode.size as usize)
    }

    pub fn inode_info(&self) -> (usize, usize) {
        let _fs = self.fs.lock();
        (self.block_id, self.block_offset)
    }

    // 包括 find 在内, 所有暴露给文件系统的使用者的文件系统操作(还包括接下来将要介绍的几种),
    // 全程均需持有 EasyFileSystem 的互斥锁
    // (相对而言, 文件系统内部的操作, 如之前的 Inode::new 或是上面的 find_inode_id ,
    // 都是假定在已持有 efs 锁的情况下才被调用的, 因此它们不应尝试获取锁).
    // 这能够保证在多核情况下, 同时最多只能有一个核在进行文件系统相关操作.

    // 文件列举
    // ls 方法可以收集目录下的所有文件的文件名并以向量的形式返回,
    pub fn ls(&self) -> Vec<String> {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            let file_count = (disk_inode.size as usize) / DIRENT_SIZE;
            let mut v: Vec<String> = Vec::new();
            for i in 0..file_count {
                let mut dir_entry = DirEntry::create_empty();
                assert_eq!(
                    disk_inode.read_at(
                        DIRENT_SIZE * i,
                        dir_entry.as_bytes_mut(),
                        &self.block_device,
                    ),
                    DIRENT_SIZE,
                );
                v.push(String::from(dir_entry.name()));
            }
            v
        })
    }

    // 文件创建
    // create 方法可以在目录下创建一个文件
    // 返回 文件的 Inode
    pub fn create(&self, name: &str, kind: DiskInodeType) -> Option<Arc<Inode>> {
        let mut fs = self.fs.lock();
        if self
            .modify_disk_inode(|disk_inode| {
                assert!(disk_inode.is_dir());
                self.find_inode_id(name, disk_inode)
            })
            .is_some()
        // 如果已经存在, 则返回 None
        {
            println!("file {} already exists", name);
            return None;
        }

        // 为新文件分配一个 inode 编号
        let new_inode_id = fs.alloc_inode();
        let (new_inode_block_id, new_inode_block_offset) = fs.get_disk_inode_pos(new_inode_id);

        get_block_cache(new_inode_block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(new_inode_block_offset, |new_inode: &mut DiskInode| {
                if kind == DiskInodeType::File {
                    new_inode.initialize(DiskInodeType::File);
                } else {
                    new_inode.initialize(DiskInodeType::Directory);
                }
            });

        // 将待创建文件的目录项插入到目录的内容中, 使得之后可以索引到
        self.modify_disk_inode(|disk_inode| {
            // 在目录中添加一个目录项
            let file_count = (disk_inode.size as usize) / DIRENT_SIZE;
            let new_size = (file_count + 1) * DIRENT_SIZE;
            // 增加目录的大小
            self.increase_size(new_size as u32, disk_inode, &mut fs);
            // 在目录的最后添加一个目录项
            let dir_entry = DirEntry::new(name, new_inode_id as u32);
            disk_inode.write_at(
                // 在此处开始写一个目录项,  大小为 DIRENT_SIZE,  最后root_inode的大小为 new_size
                file_count * DIRENT_SIZE,
                dir_entry.as_bytes(),
                &self.block_device,
            );
        });

        // Q: 这与上面的 new_inode_block_id, new_inode_block_offset 有什么区别?
        // let (block_id, block_offset) = fs.get_disk_inode_pos(new_inode_id);

        block_cache_sync_all();

        Some(Arc::new(Self::new(
            new_inode_block_id,
            new_inode_block_offset,
            self.fs.clone(),
            self.block_device.clone(),
        )))
    }

    fn increase_size(
        &self,
        new_size: u32,
        disk_inode: &mut DiskInode,
        fs: &mut MutexGuard<FileSystem>,
    ) {
        if new_size < disk_inode.alloc_size {
            // fix: bug
            // 某种操作后(可能为 删除文件夹下一个有数据的文件)无法创建文件
            disk_inode.size = new_size;
            return;
        }

        let blocks_needed = disk_inode.blocks_num_needed(new_size);
        let mut v: Vec<u32> = Vec::new();
        for _ in 0..blocks_needed {
            v.push(fs.alloc_data());
        }
        disk_inode.increase_size(new_size, v, &self.block_device);
    }

    // 文件删除
    // 在以某些标志位打开文件(例如带有 CREATE 标志打开一个已经存在的文件)的时候, 需要首先将文件清空.
    // 在索引到文件的 Inode 之后, 可以调用 clear 方法
    // 将该文件占据的索引块和数据块回收
    pub fn clear(&self) {
        let mut fs = self.fs.lock();
        self.modify_disk_inode(|disk_inode| {
            let size = disk_inode.alloc_size;
            let data_blocks_dealloc = disk_inode.clear_size(&self.block_device);

            assert!(data_blocks_dealloc.len() == DiskInode::total_blocks(size) as usize);

            for data_block in data_blocks_dealloc.into_iter() {
                fs.dealloc_data(data_block);
            }
        });

        block_cache_sync_all();
    }

    /// 删除目录项
    //
    // 类似删除顺序表的某个元素
    // 这个方法感觉不是很好 时间复杂度O(n) 空间复杂度O(n)
    pub fn rm_dir_entry(&self, file_name: &str, parent_inode: Arc<Inode>) {
        let _fs = self.fs.lock();

        // 找到dir_entry_pos
        let pos = parent_inode.dir_entry_pos(file_name); // 提前找到位置, 防止拿不到锁
        if pos.is_none() {
            println!("rm_dir_entry: file not found");
            return;
        }
        let pos = pos.unwrap();
        parent_inode.modify_disk_inode(|disk_inode| {
            let file_count = (disk_inode.size as usize) / DIRENT_SIZE;
            let new_size = (file_count - 1) * DIRENT_SIZE;

            // 从pos开始, 将后面的dir_entry往前移动
            let mut dir_entry_list: Vec<DirEntry> = Vec::new();

            // 为什么不合并: 读写冲突
            // fix:
            for i in (pos + 1)..file_count {
                let mut dir_entry = DirEntry::create_empty();
                assert_eq!(
                    disk_inode.read_at(
                        i * DIRENT_SIZE,
                        dir_entry.as_bytes_mut(),
                        &self.block_device,
                    ),
                    DIRENT_SIZE,
                );
                dir_entry_list.push(dir_entry);
            }

            for i in pos..(file_count - 1) {
                let dir_entry = dir_entry_list.remove(0);
                assert_eq!(
                    disk_inode.write_at(i * DIRENT_SIZE, dir_entry.as_bytes(), &self.block_device),
                    DIRENT_SIZE,
                );
            }

            // 将最后一个dir_entry清空
            let dir_entry = DirEntry::create_empty();
            disk_inode.write_at(
                (file_count - 1) * DIRENT_SIZE,
                dir_entry.as_bytes(),
                &self.block_device,
            );

            // 修改size (ps: 可以去看看 layout::write 处提到的 bug-fix)
            disk_inode.size = new_size as u32;
        });

        block_cache_sync_all();
    }

    fn dir_entry_pos(&self, file_name: &str) -> Option<usize> {
        self.read_disk_inode(|disk_inode| -> Option<usize> {
            let file_count = (disk_inode.size as usize) / DIRENT_SIZE;
            for i in 0..file_count {
                let mut dir_entry = DirEntry::create_empty();
                assert_eq!(
                    disk_inode.read_at(
                        i * DIRENT_SIZE,
                        dir_entry.as_bytes_mut(),
                        &self.block_device
                    ),
                    DIRENT_SIZE
                );
                if dir_entry.name() == file_name {
                    return Some(i);
                }
            }
            None
        })
    }

    // 文件读写
    //从目录索引到一个文件之后, 可以对它进行读写.
    // 注意: 和 DiskInode 一样, 这里的读写作用在字节序列的一段区间上

    pub fn read(&self, offset: usize, buf: &mut [u8]) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| disk_inode.read_at(offset, buf, &self.block_device))
    }

    pub fn chname(&self, old_name: &str, new_name: &str) {
        let _fs = self.fs.lock();

        self.modify_disk_inode(|curr_inode| {
            // find file by name
            let file_count = (curr_inode.alloc_size as usize) / DIRENT_SIZE;
            let mut dir_entry = DirEntry::create_empty();

            // BUG(disk_inode.size): 之后的文件无法读取 -> write change size

            for i in 0..file_count {
                curr_inode.read_at(
                    i * DIRENT_SIZE,
                    dir_entry.as_bytes_mut(),
                    &self.block_device,
                );
                if dir_entry.name() == old_name {
                    dir_entry.chname(new_name);
                    curr_inode.write_at(i * DIRENT_SIZE, dir_entry.as_bytes(), &self.block_device);
                    break;
                }
            }
        });
        // fix: 此时退出文件 cache 未同步, 再次打开时不会被修改(事实上可以在 main.rs 的 exit 中同步))
        block_cache_sync_all();
    }

    pub fn dist_inode_info(&self) {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            println!("🐳 alloc_size: {} B.", disk_inode.alloc_size);
            println!("🐳 size: {} B.", disk_inode.size);
            println!("🐳 type: {:?}.", disk_inode.type_);
            println!("🐳 direct blocks: {:?}.", disk_inode.direct);
            println!("🐳 indirect1 block: {}.", disk_inode.indirect1);
            println!("🐳 indirect2 block: {}.", disk_inode.indirect2);
        });
    }

    pub fn write(&self, offset: usize, buf: &[u8]) -> usize {
        let mut fs = self.fs.lock();
        let size = self.modify_disk_inode(|disk_inode| -> usize {
            if !disk_inode.is_file() {
                error!("write to a non-file inode");
                return 0;
            }

            // 如果写入的数据超过了文件的大小, 则需要增加文件的大小
            self.increase_size((offset + buf.len()) as u32, disk_inode, &mut fs);
            // 写入数据
            let write_size = disk_inode.write_at(offset, buf, &self.block_device);

            // 修改size (ps: 可以去看看 layout::write 处提到的bug-fix)
            disk_inode.size = (offset + write_size) as u32;

            write_size
        });
        block_cache_sync_all();
        size
    }
}
