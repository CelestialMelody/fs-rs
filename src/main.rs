use std::{
    fs::{read_dir, File, OpenOptions},
    io::{stdin, stdout, Read, Seek, SeekFrom, Write},
    sync::{Arc, Mutex},
};

use clap::{App, Arg};

use fs::{BlockDevice, EasyFileSystem, BLOCK_SIZE};

mod cell;
mod fs;

use lazy_static::*;

use crate::cell::UnSafeCell;

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

const USER: &str = "Clstilmldy";

lazy_static! {
    static ref PATH: UnSafeCell<String> =
        unsafe { UnSafeCell::new(format!("❂ {}   ~\n╰─❯ ", USER)) };
}

fn update_path(target: &str) {
    match target {
        // 如果是 target == ""
        "" => {
            PATH.borrow_mut().clear();
            PATH.borrow_mut()
                .push_str(&format!("❂ {}   ~\n╰─❯ ", USER));
        }
        // 如果targer == "."
        "." => return,
        // 如果target == ".."
        ".." => {
            // 获取当前路径
            let mut path = PATH.borrow_mut();
            // 如果当前路径是根目录
            if *path == format!("❂ {}   ~\n╰─❯ ", USER) {
                // 直接返回
                return;
            }
            // 如果当前路径不是根目录
            // 获取当前路径的最后一个"/"的位置
            let pos = path.rfind('/').unwrap();
            // 如果当前路径的最后一个"/"的位置不是根目录
            // 将当前路径设置为当前路径的最后一个"/"的位置
            path.replace_range(pos.., "");
            path.push_str("\n╰─❯ ");
        }
        _ => {
            let idx = PATH.borrow().find('\n').unwrap();
            let mut path = PATH.borrow_mut();
            path.drain(idx..);
            path.push_str(format!("/{}\n╰─❯ ", target).as_str());
        }
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
        .arg(
            // target 参数
            Arg::with_name("ways to run")
                .short("w")
                .long("ways")
                .takes_value(true)
                .help("Executable ways use \"create\" or \"open\""),
        )
        .get_matches();

    let src_path = matche.value_of("source").unwrap();
    let target_path = matche.value_of("target").unwrap();

    if !target_path.ends_with('/') && !src_path.ends_with('/') {
        // 如果target_path 最后一个字符不是"/"
        panic!("src_path / target_path must end with '/'");
    };

    let ways = matche.value_of("ways to run").unwrap();

    // 创建虚拟块设备
    // 打开虚拟块设备。这里我们在 Linux 上创建文件 ./target/fs.img 来新建一个虚拟块设备，并将它的容量设置为 0x4000 个块。
    // 在创建的时候需要将它的访问权限设置为可读可写。
    let block_file = Arc::new(BlockFile(Mutex::new({
        // 创建 / 打开文件，设置权限
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(format!("{}fs.img", target_path))?;
        // 设置文件大小
        f.set_len((BLOCK_NUM * BLOCK_SIZE) as u64).unwrap();
        f
    })));

    let efs = if ways == "create" {
        // 在虚拟块设备 block_file 上初始化 easy-fs 文件系统
        let efs = EasyFileSystem::create(block_file.clone(), BLOCK_NUM as u32, 1);
        efs
    } else if ways == "open" {
        // 在虚拟块设备 block_file 上打开 easy-fs 文件系统
        let efs = EasyFileSystem::open(block_file.clone());
        efs
    } else {
        panic!("Please specify the operation(create or open)!");
    };

    // 读取目录
    let root_inode = Arc::new(EasyFileSystem::root_inode(&efs));

    // 创建文件系统时
    if ways == "create" {}

    loop {
        // shell display
        print!("{}", PATH.borrow());
        stdout().flush().expect("Failed to flush stdout :(");

        // Take in user input
        let mut input = String::new();
        stdin()
            .read_line(&mut input)
            .expect("Failed to read input :(");

        // Split input into command and args
        let mut input = input.trim().split_whitespace(); // Shadows String with SplitWhitespace Iterator
        let cmd = input.next().unwrap();
        match cmd {
            "cd" => {
                update_path(input.next().unwrap_or(""));
            }

            // 读取目录下的所有文件
            "ls" => {
                for file in root_inode.ls() {
                    // 从easy-fs中读取文件
                    println!("{}", file);
                }
            }

            // 从 easy-fs 读取文件保存到 host 文件系统中
            "get" => {
                for file in root_inode.ls() {
                    // 从easy-fs中读取文件
                    println!("get {} from easy-fs", file);
                    let inode = root_inode.find(file.as_str()).unwrap();
                    let mut all_data: Vec<u8> = vec![0; inode.file_size() as usize];
                    inode.read(0, &mut all_data);
                    // 写入文件 保存到host文件系统中
                    let mut target_file = File::create(format!("{}{}", target_path, file)).unwrap();
                    target_file.write_all(all_data.as_slice()).unwrap();
                }
            }

            // 读取 src_path 下的所有文件 保存到 easy-fs 中
            "set" => {
                let files: Vec<_> = read_dir(src_path)
                    .unwrap()
                    .into_iter()
                    .map(|dir_entry| {
                        let name = dir_entry.unwrap().file_name().into_string().unwrap();
                        name
                    })
                    .collect();

                for file in files {
                    // 从host文件系统中读取文件
                    println!("set {} to easy-fs", src_path);
                    let mut host_file = File::open(format!("{}{}", src_path, file)).unwrap();
                    let mut all_data: Vec<u8> = Vec::new();
                    host_file.read_to_end(&mut all_data).unwrap();
                    // 创建文件
                    let inode = root_inode.create(file.as_str());
                    if inode.is_some() {
                        // 写入文件
                        let inode = inode.unwrap();
                        inode.write(0, all_data.as_slice());
                    }
                }
            }

            "exit" => break,
            _ => println!("Unknown command: {}", cmd),
        }
    }

    Ok(())
}

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
