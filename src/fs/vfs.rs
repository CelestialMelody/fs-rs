//! EasyFileSystem 实现了磁盘布局并能够将磁盘块有效的管理起来。
//! 但是对于文件系统的使用者而言，他们往往不关心磁盘布局是如何实现的，而是更希望能够直接看到目录树结构中逻辑上的文件和目录。
//! 为此需要设计索引节点 Inode 暴露给文件系统的使用者，让他们能够直接对文件和目录进行操作。
//!
//!  DiskInode 放在磁盘块中比较固定的位置，而 Inode 是放在内存中的记录文件索引节点信息的数据结构

use std::sync::Arc;

use crate::fs::{DirEntry, DIRENT_SIZE};

use ::log::error;

use super::{
    block_cache_sync_all, easy_fs::EasyFileSystem, get_block_cache, BlockDevice, DiskInode,
    DiskInodeType,
};

use spin::{Mutex, MutexGuard};

pub struct Inode {
    /// 位于哪个盘块
    block_id: usize,
    /// 盘块上的偏移
    block_offset: usize,
    fs: Arc<Mutex<EasyFileSystem>>,
    block_device: Arc<dyn BlockDevice>,
}

impl Inode {
    pub fn new(
        block_id: u32,
        block_offset: usize,
        fs: Arc<Mutex<EasyFileSystem>>,
        block_device: Arc<dyn BlockDevice>,
    ) -> Self {
        Self {
            block_id: block_id as usize,
            block_offset,
            fs,
            block_device,
        }
    }

    // 仿照 BlockCache::read/modify ，
    // 我们可以设计两个方法来简化对于 Inode 对应的磁盘上的 DiskInode 的访问流程，
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
    // 目前：
    // 在目录树上仅有一个目录——那就是作为根节点的根目录。所有的文件都在根目录下面。
    // 于是，我们不必实现目录索引。
    // 文件索引的查找比较简单，仅需在根目录的目录项中根据文件名找到文件的 inode 编号即可。
    // 由于没有子目录的存在，这个过程只会进行一次

    /// 根据名称查找磁盘 inode 下的 inode
    /// 尝试从根目录的 DiskInode 上找到要索引的文件名对应的 inode 编号
    fn find_inode_id(&self, name: &str, disk_inode: &DiskInode) -> Option<u32> {
        assert!(disk_inode.is_dir()); // 一定是目录
        let file_count = (disk_inode.size as usize) / DIRENT_SIZE;
        let mut dir_entry = DirEntry::create_empty();
        for i in 0..file_count {
            assert_eq!(
                disk_inode.read(
                    DIRENT_SIZE * i,
                    dir_entry.as_bytes_mut(),
                    &self.block_device,
                ),
                DIRENT_SIZE,
            ); // 读取目录项

            // 将根目录内容中的所有目录项都读到内存进行逐个比对
            // 如果能够找到，则 find 方法会根据查到 inode 编号，对应生成一个 Inode 用于后续对文件的访问
            if dir_entry.name() == name {
                return Some(dir_entry.inode_id() as u32);
            }
        }
        None
    }

    // find 方法只会被根目录 Inode 调用，文件系统中其他文件的 Inode 不会调用这个方法
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

    pub fn size(&self) -> usize {
        self.read_disk_inode(|disk_inode| disk_inode.size as usize)
    }

    // 包括 find 在内，所有暴露给文件系统的使用者的文件系统操作（还包括接下来将要介绍的几种），
    // 全程均需持有 EasyFileSystem 的互斥锁
    // （相对而言，文件系统内部的操作，如之前的 Inode::new 或是上面的 find_inode_id ，
    // 都是假定在已持有 efs 锁的情况下才被调用的，因此它们不应尝试获取锁）。
    // 这能够保证在多核情况下，同时最多只能有一个核在进行文件系统相关操作。

    // 文件列举
    // ls 方法可以收集根目录下的所有文件的文件名并以向量的形式返回，
    // 这个方法只有根目录的 Inode 才会调用
    pub fn ls(&self) -> Vec<String> {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            let file_count = (disk_inode.size as usize) / DIRENT_SIZE;
            let mut v: Vec<String> = Vec::new();
            for i in 0..file_count {
                let mut dir_entry = DirEntry::create_empty();
                assert_eq!(
                    disk_inode.read(
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
    // create 方法可以在根目录下创建一个文件，该方法只有根目录的 Inode 会调用
    pub fn create(&self, name: &str) -> Option<Arc<Inode>> {
        let mut fs = self.fs.lock();
        if self
            .modify_disk_inode(|curr_inode| {
                assert!(curr_inode.is_dir());
                self.find_inode_id(name, curr_inode)
            })
            .is_some()
        // 如果已经存在，则返回 None
        {
            return None;
        }

        // 为新文件分配一个 inode 编号
        let new_inode_id = fs.alloc_inode();
        let (new_inode_block_id, new_inode_block_offset) = fs.get_disk_inode_pos(new_inode_id);

        get_block_cache(new_inode_block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(new_inode_block_offset, |new_inode: &mut DiskInode| {
                new_inode.initialize(DiskInodeType::File);
            });

        // 将待创建文件的目录项插入到根目录的内容中，使得之后可以索引到
        self.modify_disk_inode(|root_inode| {
            // 在根目录中添加一个目录项
            let file_count = (root_inode.size as usize) / DIRENT_SIZE;
            let new_size = (file_count + 1) * DIRENT_SIZE;
            // 增加根目录的大小
            self.increase_size(new_size as u32, root_inode, &mut fs);
            // 在根目录的最后添加一个目录项
            // 返回目录 inode_id
            let dir_entry = DirEntry::new(name, new_inode_id as u32, {
                let parent_inode_id = if {
                    let self_block_id = self.block_id;
                    let self_block_offset = self.block_offset;
                    // root inode_id is 0
                    let (block_id, block_offset) = fs.get_disk_inode_pos(0);

                    let is_root = if self_block_id == block_id as usize
                        && self_block_offset == block_offset
                    {
                        true
                    } else {
                        false
                    };
                    is_root
                } {
                    0
                } else {
                    self.read_disk_inode(|disk_inode| -> u32 {
                        let mut dir_entry = DirEntry::create_empty();
                        assert_eq!(
                            disk_inode.read(0, dir_entry.as_bytes_mut(), &self.block_device),
                            DIRENT_SIZE,
                        );
                        dir_entry.parent_inode_id()
                    })
                };
                parent_inode_id
            });
            root_inode.write(
                // 在此处开始写一个目录项， 大小为 DIRENT_SIZE， 最后root_inode的大小为 new_size
                file_count * DIRENT_SIZE,
                dir_entry.as_bytes(),
                &self.block_device,
            );
        });

        // Q: 这与上面的 new_inode_block_id, new_inode_block_offset 有什么区别？
        let (block_id, block_offset) = fs.get_disk_inode_pos(new_inode_id);

        block_cache_sync_all();

        Some(Arc::new(Self::new(
            block_id,
            block_offset,
            self.fs.clone(),
            self.block_device.clone(),
        )))
    }

    // pub fn parent_inode_id(&self) -> u32 {
    //     if self.is_root() {
    //         return 0;
    //     }

    //     self.read_disk_inode(|disk_inode| -> u32 {
    //         let mut dir_entry = DirEntry::create_empty();
    //         assert_eq!(
    //             disk_inode.read(0, dir_entry.as_bytes_mut(), &self.block_device),
    //             DIRENT_SIZE,
    //         );
    //         dir_entry.parent_inode_id()
    //     })
    // }

    // pub fn inode_id(&self) -> u32 {
    //     self.read_disk_inode(|disk_inode| -> u32 {
    //         let mut dir_entry = DirEntry::create_empty();
    //         assert_eq!(
    //             disk_inode.read(0, dir_entry.as_bytes_mut(), &self.block_device),
    //             DIRENT_SIZE,
    //         );
    //         dir_entry.inode_id()
    //     })
    // }

    fn increase_size(
        &self,
        new_size: u32,
        disk_inode: &mut DiskInode,
        fs: &mut MutexGuard<EasyFileSystem>,
    ) {
        if new_size < disk_inode.cap_size {
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
    // 在以某些标志位打开文件（例如带有 CREATE 标志打开一个已经存在的文件）的时候，需要首先将文件清空。
    // 在索引到文件的 Inode 之后，可以调用 clear 方法
    // 将该文件占据的索引块和数据块回收
    pub fn clear(&self) {
        let mut fs = self.fs.lock();
        self.modify_disk_inode(|disk_inode| {
            let size = disk_inode.cap_size;
            let data_blocks_dealloc = disk_inode.clear_size(&self.block_device);

            assert!(data_blocks_dealloc.len() == DiskInode::total_blocks(size) as usize);

            for data_block in data_blocks_dealloc.into_iter() {
                fs.dealloc_data(data_block);
            }
        });

        block_cache_sync_all();
    }

    /// 非根目录调用
    pub fn rm_dir_entry(&self, file_name: &str) {
        // 如果要删除自身，需要获取父亲 inode
        // 将自己从父亲 inode 中删除
        // 目前想法是
        // 1. 内存中维护 curr_inode 与 parent_inode;
        // 或者修改 DirEntry, 添加 parent_inode_id 通过 parent_inode_id 找到 parent_inode (x)
        // 2. 不可以直接回收 block 清理block，不能保证 block_id 对应的 block 没有文件了
        // 3. 可能需要实现 decrease_size
        // 简单的想法是 回收一个 目录项 -> 改变目录大小，
        // TODO: 当目录大小为 0 时（DiskInode size），回收目录 block
        // TODO
        let fs = self.fs.lock();

        //
        let parent_inode_id = if {
            let self_block_id = self.block_id;
            let self_block_offset = self.block_offset;
            // root inode_id is 0
            let (block_id, block_offset) = fs.get_disk_inode_pos(0);

            let is_root = if self_block_id == block_id as usize && self_block_offset == block_offset
            {
                true
            } else {
                false
            };
            is_root
        } {
            0
        } else {
            // bug: 已经清除了目录项，没法获取父亲 inode_id
            self.read_disk_inode(|disk_inode| -> u32 {
                let mut dir_entry = DirEntry::create_empty();
                assert_eq!(
                    disk_inode.read(0, dir_entry.as_bytes_mut(), &self.block_device),
                    DIRENT_SIZE,
                );
                dir_entry.parent_inode_id()
            })
        };

        //

        let (block_id, block_offset) = fs.get_disk_inode_pos(parent_inode_id);
        let parent_inode = Arc::new(Self::new(
            block_id,
            block_offset,
            self.fs.clone(),
            self.block_device.clone(),
        ));
        parent_inode.modify_disk_inode(|disk_inode| {
            let file_count = (disk_inode.size as usize) / DIRENT_SIZE;
            let new_size = (file_count - 1) * DIRENT_SIZE;

            // 找到dir_entry_pos
            let pos = self.dir_entry_pos(file_name);
            if pos.is_none() {
                println!("rm_dir_entry: file not found");
                return;
            }
            let pos = pos.unwrap();

            // 从pos开始，将后面的dir_entry往前移动
            for i in pos..file_count - 1 {
                let mut dir_entry = DirEntry::create_empty();
                assert_eq!(
                    disk_inode.read(
                        (i + 1) * DIRENT_SIZE,
                        dir_entry.as_bytes_mut(),
                        &self.block_device,
                    ),
                    DIRENT_SIZE,
                );
                assert_eq!(
                    disk_inode.write(i * DIRENT_SIZE, dir_entry.as_bytes(), &self.block_device),
                    DIRENT_SIZE,
                );
            }

            // 将最后一个dir_entry清空
            let dir_entry = DirEntry::create_empty();
            disk_inode.write(
                (file_count - 1) * DIRENT_SIZE,
                dir_entry.as_bytes(),
                &self.block_device,
            );

            // 修改size
            disk_inode.size = new_size as u32;
        });
    }

    fn dir_entry_pos(&self, file_name: &str) -> Option<usize> {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| -> Option<usize> {
            let file_count = (disk_inode.size as usize) / DIRENT_SIZE;
            for i in 0..file_count {
                let mut dir_entry = DirEntry::create_empty();
                assert_eq!(
                    disk_inode.read(
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
    //从根目录索引到一个文件之后，可以对它进行读写。
    // 注意：和 DiskInode 一样，这里的读写作用在字节序列的一段区间上

    pub fn read(&self, offset: usize, buf: &mut [u8]) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| disk_inode.read(offset, buf, &self.block_device))
    }

    pub fn chname(&self, old_name: &str, new_name: &str) {
        let _fs = self.fs.lock();

        self.modify_disk_inode(|curr_inode| {
            // find file by name
            let file_count = (curr_inode.size as usize) / DIRENT_SIZE;
            let mut dir_entry = DirEntry::create_empty();
            for i in 0..file_count {
                curr_inode.read(
                    i * DIRENT_SIZE,
                    dir_entry.as_bytes_mut(),
                    &self.block_device,
                );
                if dir_entry.name() == old_name {
                    dir_entry.chname(new_name);
                    curr_inode.write(i * DIRENT_SIZE, dir_entry.as_bytes(), &self.block_device);
                    break;
                }
            }
        })
    }

    pub fn write(&self, offset: usize, buf: &[u8]) -> usize {
        let mut fs = self.fs.lock();
        let size = self.modify_disk_inode(|disk_inode| -> usize {
            if !disk_inode.is_file() {
                error!("write to a non-file inode");
                return 0;
            }

            // 如果写入的数据超过了文件的大小，则需要增加文件的大小
            self.increase_size((offset + buf.len()) as u32, disk_inode, &mut fs);
            // 写入数据
            disk_inode.write(offset, buf, &self.block_device)
        });
        block_cache_sync_all();
        size
    }

    // 由于 根目录 没有 DirEntry，因此需要特殊处理
    // fn is_root(&self) -> bool {
    //     let fs = self.fs.lock();
    //     let self_block_id = self.block_id;
    //     let self_block_offset = self.block_offset;
    //     // root inode_id is 0
    //     let (block_id, block_offset) = fs.get_disk_inode_pos(0);

    //     if self_block_id == block_id as usize && self_block_offset == block_offset {
    //         return true;
    //     }
    //     false
    // }

    // TODO 目录索引
}
