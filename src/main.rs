use std::{
    fs::{read_dir, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    sync::{Arc, Mutex},
};

use clap::{App, Arg};

use fs::{BlockDevice, EasyFileSystem, BLOCK_SIZE};

mod fs;

const BLOCK_NUM: usize = 0x4000;

struct BlockFile(Mutex<File>);

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

fn main() {
    easy_fs_pack().expect("Error when packing easy fs");
}

fn easy_fs_pack() -> std::io::Result<()> {
    // 从命令行参数中获取文件名
    let matche = App::new("EasyFileSystem Packer")
        .arg(
            // source 参数
            Arg::with_name("source")
                .short("s")
                .long("source")
                .takes_value(true)
                .help("Executable source dir(with backslash '/')"),
        )
        .arg(
            // target 参数
            Arg::with_name("target")
                .short("t")
                .long("target")
                .takes_value(true)
                .help("Executable target dir(with backslash '/')"),
        )
        .get_matches();

    // let src_path = matche.value_of("source").unwrap_or("/");
    let src_path = matche.value_of("source").unwrap();
    let target_path = matche.value_of("target").unwrap();
    println!("src_path: {}\ntarget_path: {}", src_path, target_path);

    // 创建虚拟块设备
    // 打开虚拟块设备。这里我们在 Linux 上创建文件 ./target/fs.img 来新建一个虚拟块设备，并将它的容量设置为 0x4000 个块。
    // 在创建的时候需要将它的访问权限设置为可读可写。
    let block_file = Arc::new(BlockFile(Mutex::new({
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(format!("{}{}", target_path, "fs.img"))?;
        f.set_len((BLOCK_NUM * BLOCK_SIZE) as u64).unwrap();
        f
    })));

    // 在虚拟块设备 block_file 上初始化 easy-fs 文件系统
    let efs = EasyFileSystem::create(block_file.clone(), BLOCK_NUM as u32, 1);

    // 读取目录
    let root_inode = Arc::new(EasyFileSystem::root_inode(&efs));

    // 读取目录下的所有文件
    let apps: Vec<_> = read_dir(src_path)
        .unwrap()
        .into_iter()
        .map(|dir_entry| {
            // let mut name_with_ext = dir_entry.unwrap().file_name().into_string().unwrap();
            let name_with_ext = dir_entry.unwrap().file_name().into_string().unwrap();
            // name_with_ext.drain(name_with_ext.find('.').unwrap()..name_with_ext.len());
            name_with_ext // name without ext
        })
        .collect();

    for app in apps {
        // 从host文件系统中读取文件
        let mut host_file = File::open(format!("{}{}", target_path, app)).unwrap();
        let mut all_data: Vec<u8> = Vec::new();
        host_file.read_to_end(&mut all_data).unwrap();
        // 创建文件
        let inode = root_inode.create(app.as_str()).unwrap();
        inode.write(0, all_data.as_slice());
    }

    // 列出目录下的所有文件
    for app in root_inode.ls() {
        println!("{}", app);
    }

    Ok(())
}

#[test]
fn efs_test() -> std::io::Result<()> {
    let block_file = Arc::new(BlockFile(Mutex::new({
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("target/fs.img")?;
        f.set_len((BLOCK_NUM * BLOCK_SIZE) as u64).unwrap();
        f
    })));
    EasyFileSystem::create(block_file.clone(), 4096, 1);
    let efs = EasyFileSystem::open(block_file.clone());
    let root_inode = EasyFileSystem::root_inode(&efs);
    root_inode.create("filea");
    root_inode.create("fileb");
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
