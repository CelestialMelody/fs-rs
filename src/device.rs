use crate::fs::{BlockDevice, BLOCK_SIZE};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    sync::Mutex,
};
pub struct BlockFile(pub Mutex<File>);

// std::file::File 由 Rust 标准库 std 提供，可以访问 Linux 上的一个文件。
// 我们将它包装成 BlockFile 类型来模拟一块磁盘，为它实现 BlockDevice 接口。
// 注意 File 本身仅通过 read/write 接口是不能实现随机读写的，
// 在访问一个特定的块的时候，我们必须先 seek 到这个块的开头位置

impl BlockDevice for BlockFile {
    /// 读取一个块从文件
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        let mut file = self.0.lock().unwrap();
        file.seek(SeekFrom::Start((block_id * BLOCK_SIZE) as u64))
            .expect("Error when seeking!");
        assert_eq!(file.read(buf).unwrap(), BLOCK_SIZE, "Not a complete block");
    }

    /// 写一个块到文件
    fn write_block(&self, block_id: usize, buf: &[u8]) {
        let mut file = self.0.lock().unwrap();
        file.seek(SeekFrom::Start((block_id * BLOCK_SIZE) as u64))
            .expect("Error when seeking!");
        assert_eq!(file.write(buf).unwrap(), BLOCK_SIZE, "Not a complete block");
    }
}
