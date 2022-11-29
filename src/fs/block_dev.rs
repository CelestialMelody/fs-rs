//! 在 easy-fs 中并没有一个实现了 BlockDevice Trait 的具体类型。
//! 因为块设备仅支持以块为单位进行随机读写，所以需要由具体的块设备驱动来实现这两个方法，
//! 实际上这是需要由文件系统的使用者（比如操作系统内核或直接测试 easy-fs 文件系统的 easy-fs-fuse 应用程序）提供并接入到 easy-fs 库的。
//! easy-fs 库的块缓存层会调用这两个方法，进行块缓存的管理。
//! 这也体现了 easy-fs 的泛用性：它可以访问实现了 BlockDevice Trait 的块设备驱动程序。

use std::any::Any;

// 块与扇区
// 实际上，块和扇区是两个不同的概念。
// 扇区 (Sector) 是块设备随机读写的数据单位，通常每个扇区为 512 字节。
// 而块是文件系统存储文件时的数据单位，每个块的大小等同于一个或多个扇区。

// 块设备接口层
// 定义设备驱动需要实现的块读写接口 BlockDevice trait

pub trait BlockDevice: Send + Sync + Any {
    // read_block 将编号为 block_id 的块从磁盘读入内存中的缓冲区 buf ；
    fn read_block(&self, block_id: usize, buf: &mut [u8]);

    // write_block 将内存中的缓冲区 buf 中的数据写入磁盘编号为 block_id 的块。
    fn write_block(&self, block_id: usize, buf: &[u8]);
}
