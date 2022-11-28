mod bitmap;
mod block_cache;
mod block_dev;
mod easy_fs;
mod layout;
mod vfs;

extern crate log;

/// Use a block size of 512 bytes
pub const BLOCK_SIZE: usize = 512;
/// 为了避免在块缓存上浪费过多内存，我们希望内存中同时只能驻留有限个磁盘块的缓冲区
pub const BLOCK_CACHE_SIZE: usize = 16;
/// Magic number for sanity check
pub const EAZY_FS_MAGIC: u32 = 0x3b800001;
/// The max number of direct inodes
pub const INODE_DIRECT_COUNT: usize = 28;
/// The max length of inode name
pub const NAME_LENGTH_LIMIT: usize = 27;
/// The max number of indirect1 inodes
pub const INODE_INDIRECT1_COUNT: usize = BLOCK_SIZE / 4;
/// The max number of indirect2 inodes
pub const INODE_INDIRECT2_COUNT: usize = INODE_INDIRECT1_COUNT * INODE_INDIRECT1_COUNT;
/// The upper bound of direct inode index
pub const DIRECT_BOUND: usize = INODE_DIRECT_COUNT;
/// The upper bound of indirect1 inode index
pub const INDIRECT1_BOUND: usize = DIRECT_BOUND + INODE_INDIRECT1_COUNT;
/// The upper bound of indirect2 inode index
#[allow(unused)]
pub const INDIRECT2_BOUND: usize = INDIRECT1_BOUND + INODE_INDIRECT2_COUNT;
/// 块的 bit 数量
pub const BLOCK_BITS: usize = BLOCK_SIZE * 8;
/// 目录项的大小
pub const DIRENT_SIZE: usize = 32;

pub use bitmap::Bitmap;
pub use block_cache::{block_cache_sync_all, get_block_cache};
pub use block_dev::BlockDevice;
pub use easy_fs::EasyFileSystem;
pub use layout::*;
pub use vfs::Inode;
