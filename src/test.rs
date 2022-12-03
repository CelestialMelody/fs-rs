#![allow(unused)]
use super::device;
use super::fs;
use crate::fs::DirEntry;
use crate::BLOCK_NUM;
use device::BlockFile;
use fs::{BlockDevice, EasyFileSystem, BLOCK_SIZE};
use std::fs::OpenOptions;
use std::sync::{Arc, Mutex};

#[test]
fn efs_test() -> std::io::Result<()> {
    // 创建虚拟磁盘
    let block_file = Arc::new(BlockFile(Mutex::new({
        // 创建文件，设置权限
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("target/fs.img")?;
        // 设置文件大小
        f.set_len((BLOCK_NUM * BLOCK_SIZE) as u64).unwrap();
        f
    })));

    // 在虚拟块设备 block_file 上初始化 easy-fs 文件系统
    EasyFileSystem::create(block_file.clone(), 4096, 1);

    // 打开文件系统
    let efs = EasyFileSystem::open(block_file.clone());

    // 读取根目录
    let root_inode = EasyFileSystem::root_inode(&efs);

    root_inode.create("filea", fs::DiskInodeType::File);
    root_inode.create("fileb", fs::DiskInodeType::File);
    for name in root_inode.ls() {
        println!("{}", name);
    }

    let filea = root_inode.find("filea").unwrap();

    let greet_str = "Hello, world!";
    filea.write(0, greet_str.as_bytes());
    //let mut buffer = [0u8; BLOCK_SIZE];
    let mut buffer = [0u8; 233];
    let len = filea.read(0, &mut buffer);
    assert_eq!(greet_str, core::str::from_utf8(&buffer[..len]).unwrap(),);

    let mut random_str_test = |len: usize| {
        filea.clear();
        assert_eq!(filea.read(0, &mut buffer), 0,);
        let mut str = String::new();
        use rand;
        // random digit
        for _ in 0..len {
            str.push(char::from('0' as u8 + rand::random::<u8>() % 10));
        }
        filea.write(0, str.as_bytes());
        let mut read_buffer = [0u8; 127];
        let mut offset = 0usize;
        let mut read_str = String::new();
        loop {
            let len = filea.read(offset, &mut read_buffer);
            if len == 0 {
                break;
            }
            offset += len;
            read_str.push_str(core::str::from_utf8(&read_buffer[..len]).unwrap());
        }
        assert_eq!(str, read_str);
    };

    random_str_test(4 * BLOCK_SIZE);
    random_str_test(8 * BLOCK_SIZE + BLOCK_SIZE / 2);
    random_str_test(100 * BLOCK_SIZE);
    random_str_test(70 * BLOCK_SIZE + BLOCK_SIZE / 7);
    random_str_test((12 + 128) * BLOCK_SIZE);
    random_str_test(400 * BLOCK_SIZE);
    random_str_test(1000 * BLOCK_SIZE);
    random_str_test(2000 * BLOCK_SIZE);

    Ok(())
}
