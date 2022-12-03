use crate::cell::UnSafeCell;
use chrono::{
    format::{DelayedFormat, StrftimeItems},
    prelude::*,
};
use clap::{App, Arg};
use device::BlockFile;
use fs::{EasyFileSystem, BLOCK_SIZE};
use lazy_static::*;
use std::{
    fs::{read_dir, File, OpenOptions},
    io::{stdin, stdout, Read, Write},
    sync::{Arc, Mutex},
};

mod cell;
mod device;
mod fs;
mod test;

pub const BLOCK_NUM: usize = 0x4000;
const USER: &str = "Clstilmldy";

lazy_static! {
    static ref PATH: UnSafeCell<String> =
        unsafe { UnSafeCell::new(format!("❂ {}   ~\n╰─❯ ", USER)) };
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
    let curr_folder_inode = Arc::new(EasyFileSystem::root_inode(&efs));

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
                for file in curr_folder_inode.ls() {
                    // 从easy-fs中读取文件
                    println!("{}", file);
                }
            }

            // read filename offset size
            "read" => {
                let file_name = input.next().unwrap_or("");
                let file_inode = curr_folder_inode.find(file_name);
                if file_inode.is_none() {
                    println!("File not found!");
                    continue;
                }
                let file_inode = file_inode.unwrap();

                // 如果 input 只有一个参数，那么就是读取整个文件：offset = 0，size = 文件大小
                // 如果 input 只有两个参数，那么就是读取文件的一部分：offset = 第一个参数，size = 文件大小 - offset
                let next1 = input.next().unwrap_or("0");
                let next2 = input.next();
                if next2 == None {
                    // 读取整个文件
                    let offset = next1.parse::<usize>().unwrap();
                    let size = file_inode.size() as usize - offset;
                    let mut buf = vec![0u8; size];
                    file_inode.read(offset, &mut buf);
                    unsafe {
                        println!("{}", String::from_utf8_unchecked(buf));
                    }
                } else {
                    // 读取文件的一部分
                    let offset = next1.parse::<usize>().unwrap();
                    let size = next2.unwrap().parse::<usize>().unwrap();
                    let mut buf = vec![0u8; size];
                    file_inode.read(offset, &mut buf);
                    unsafe {
                        println!("{}", String::from_utf8_unchecked(buf));
                    }
                }

                // 因为没法保证文件的内容是可打印的( offset 开始读的地方 以及最后的长度 不保证是合法的utf8字符)
            }

            "chname" => {
                let file_name = input.next();
                if file_name.is_none() {
                    println!("Please specify the file name!");
                    continue;
                }
                let file_name = file_name.unwrap();

                let new_name = input.next();
                if new_name.is_none() {
                    println!("Please specify the new name!");
                    continue;
                }
                let new_name = new_name.unwrap();

                curr_folder_inode.chname(file_name, new_name);
            }

            // write filename offset content
            "write" => {
                let file_name = input.next().unwrap_or("");
                let file_inode = curr_folder_inode.find(file_name);
                if file_inode.is_none() {
                    println!("File not found!");
                    continue;
                }
                let file_inode = file_inode.unwrap();

                // 如果 next 不是数字
                let next = input.next().unwrap();
                if next.parse::<usize>().is_err() {
                    // 那么就是写入整个文件：offset = 0，content = 第一个参数
                    let content = next;
                    file_inode.write(0, content.as_bytes());
                } else {
                    // 如果 next 是数字
                    // 那么就是写入文件的一部分：offset = 第一个参数，content = 第二个参数
                    let offset = next.parse::<usize>().unwrap();
                    let content = input.next().unwrap_or("");
                    file_inode.write(offset, content.as_bytes());
                };
            }

            // 从 easy-fs 读取文件保存到 host 文件系统中
            "get" => {
                for file in curr_folder_inode.ls() {
                    // 从easy-fs中读取文件
                    println!("get {} from easy-fs", file);
                    let inode = curr_folder_inode.find(file.as_str()).unwrap();
                    let mut all_data: Vec<u8> = vec![0; inode.size() as usize];
                    inode.read(0, &mut all_data);
                    // 写入文件 保存到host文件系统中
                    let mut target_file = File::create(format!(
                        "{}{} {}",
                        target_path,
                        format!("{}", {
                            let fmt = "%Y-%m-%d %H:%M:%S";
                            let now: DateTime<Local> = Local::now();
                            let dft: DelayedFormat<StrftimeItems> = now.format(fmt);
                            dft.to_string()
                        },)
                        .as_str(),
                        file
                    ))
                    .unwrap();
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
                    println!("set {}{} to easy-fs", src_path, file);
                    let mut host_file = File::open(format!("{}{}", src_path, file)).unwrap();
                    let mut all_data: Vec<u8> = Vec::new();
                    host_file.read_to_end(&mut all_data).unwrap();
                    // 创建文件
                    let inode = curr_folder_inode.create(file.as_str());
                    if inode.is_some() {
                        // 写入文件
                        let inode = inode.unwrap();
                        inode.write(0, all_data.as_slice());
                    }
                }
            }

            "fmt" => {
                curr_folder_inode.clear();
            }

            "exit" => break,
            _ => println!("Unknown command: {}", cmd),
        }
    }

    Ok(())
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
