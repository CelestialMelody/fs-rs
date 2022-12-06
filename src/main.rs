use crate::{cell::UnSafeCell, fs::Inode};
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
    /// shell path
    static ref PATH: UnSafeCell<String> =
        unsafe { UnSafeCell::new(format!("❂ {}   ~\n╰─❯ ", USER)) };
}

fn main() {
    easy_fs_pack().expect("🦀 Error when packing easy fs");
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
                .help("🦀 Executable source dir(with backslash '/')"),
        )
        .arg(
            // target 参数
            Arg::with_name("target")
                .short("t")
                .long("target")
                .takes_value(true)
                .help("🦀 Executable target dir(with backslash '/')"),
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
        panic!("🦀 src_path / target_path must end with '/'");
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
        panic!("🦀 Please specify the operation(create or open)!");
    };

    // 读取目录
    let root_inode = Arc::new(EasyFileSystem::root_inode(&efs));
    let mut folder_inode: Vec<Arc<Inode>> = Vec::new();
    let mut curr_folder_inode = Arc::clone(&root_inode);

    loop {
        // shell display
        print!("{}", PATH.borrow());
        stdout().flush().expect("🦀 Failed to flush stdout :(");

        // Take in user input
        let mut input = String::new();
        stdin()
            .read_line(&mut input)
            .expect("🦀 Failed to read input :(");

        // Split input into command and args
        let mut input = input.trim().split_whitespace(); // Shadows String with SplitWhitespace Iterator
        let cmd = input.next().unwrap();
        match cmd {
            "cd" => {
                let mut copy_input = input.clone();
                let arg = copy_input.next();

                if arg.is_none() {
                    drop(curr_folder_inode);
                    curr_folder_inode = Arc::clone(&root_inode);
                } else {
                    let arg = arg.unwrap_or("");

                    // 如果 arg 以 "/" 结尾，将 target 设置为 target 的子串
                    let arg = if arg.ends_with('/') {
                        &arg[..arg.len() - 1]
                    } else {
                        arg
                    };

                    match arg {
                        "" => {
                            drop(curr_folder_inode);
                            curr_folder_inode = Arc::clone(&root_inode);
                        }
                        "." => {}
                        ".." => {
                            drop(curr_folder_inode);
                            let parent_folder_inode = folder_inode.pop();
                            if parent_folder_inode.is_none() {
                                curr_folder_inode = Arc::clone(&root_inode);
                            } else {
                                curr_folder_inode = parent_folder_inode.unwrap();
                            }
                        }
                        _ => {
                            let new_inode = curr_folder_inode.find(arg);
                            if new_inode.is_none() {
                                println!("🦀 cd: no such directory: {}! 🦐", arg);
                                continue;
                            }
                            let new_inode = new_inode.unwrap();
                            if !new_inode.is_dir() {
                                println!("🦀 cd: not a directory: {}! 🦐", arg);
                                continue;
                            }
                            folder_inode.push(Arc::clone(&curr_folder_inode));
                            drop(curr_folder_inode);
                            curr_folder_inode = new_inode;
                        }
                    }
                }

                update_path(input.next().unwrap_or(""));
            }

            "touch" => {
                let file_name = input.next();
                if file_name.is_none() {
                    println!("🦀 touch: Miss file name! 🦐");
                    continue;
                }
                let file_name = file_name.unwrap();
                curr_folder_inode.create(file_name, fs::DiskInodeType::File);
            }

            "mkdir" => {
                let file_name = input.next();
                if file_name.is_none() {
                    println!("🦀 mkdir: Miss file name! 🦐");
                    continue;
                }
                let file_name = file_name.unwrap();
                curr_folder_inode.create(file_name, fs::DiskInodeType::Directory);
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
                let file_name = input.next();
                if file_name.is_none() {
                    println!("🦀 read: Miss file name! 🦐");
                    continue;
                }
                let file_name = file_name.unwrap();
                let file_inode = curr_folder_inode.find(file_name);
                if file_inode.is_none() {
                    println!("🦀 read: File not found! 🦐");
                    continue;
                }
                let file_inode = file_inode.unwrap();
                let size = file_inode.size() as usize;

                // 如果 input 只有一个参数，那么就是读取整个文件：offset = 0，size = 文件大小
                // 如果 input 只有两个参数，那么就是读取文件的一部分：offset = 第一个参数，size = 文件大小 - offset
                let next1 = input.next().unwrap_or("0");
                let next2 = input.next();
                if next2 == None {
                    // 读取整个文件
                    let offset = next1.parse::<usize>().unwrap();
                    if size < offset {
                        println!("🦀 read: Offset is too large! 🦐");
                        continue;
                    }
                    let size = size - offset;
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

            "cat" => {
                let file_name = input.next();
                if file_name.is_none() {
                    println!("🦀 cat: Miss file name! 🦐");
                    continue;
                }
                let file_name = file_name.unwrap();
                let file_inode = curr_folder_inode.find(file_name);
                if file_inode.is_none() {
                    println!("🦀 cat: File not found! 🦐");
                    continue;
                }
                let file_inode = file_inode.unwrap();

                let mut buf = vec![0u8; file_inode.size() as usize];
                file_inode.read(0, &mut buf);
                unsafe {
                    println!("{}", String::from_utf8_unchecked(buf));
                }
            }

            "chname" => {
                let file_name = input.next();
                if file_name.is_none() {
                    println!("🦀 chname: Miss file name! 🦐");
                    continue;
                }
                let file_name = file_name.unwrap();

                let new_name = input.next();
                if new_name.is_none() {
                    println!("🦀 chname: Please specify the new name! 🦐");
                    continue;
                }
                let new_name = new_name.unwrap();

                curr_folder_inode.chname(file_name, new_name);
            }

            // write filename offset/"-a" content
            // 从 offset 开始写入 content, 只覆盖content的长度, 但我的展示方式是不让看后面的部分
            // 如果想要看后面的部分，可以去修改展示时获取的 size 为 alloc_size
            // 另外，目前写入的 content 没法换行，也就是读一串内容；
            // 如果要修改：循环读取 input，直到读到一个特殊字符
            "write" => {
                let file_name = input.next();
                if file_name.is_none() {
                    println!("🦀 write: Miss file name! 🦐");
                    continue;
                }
                let file_name = file_name.unwrap();
                let file_inode = curr_folder_inode.find(file_name);
                if file_inode.is_none() {
                    println!("🦀 write: File not found! 🦐");
                    continue;
                }
                let file_inode = file_inode.unwrap();

                // let mut size = file_inode.size();
                // 如果 next 不是数字
                // let next = input.next().unwrap();
                // if next.parse::<usize>().is_err() {
                //     // 如果是 "a" 则追加 append
                //     if next == "-a" {
                //         let context = input.next().unwrap();
                //         file_inode.write(size, context.as_bytes());
                //     } else {
                //         // 那么就是写入整个文件：offset = 0，content = 第一个参数
                //         let content = next;
                //         file_inode.write(0, content.as_bytes());
                //     }
                // } else {
                //     // 如果 next 是数字
                //     // 那么就是写入文件的一部分：offset = 第一个参数，content = 第二个参数
                //     let offset = next.parse::<usize>().unwrap();
                //     let content = input.next().unwrap_or("");
                //     if offset > size {
                //         println!("🦀 write: Offset is out of range! 🦐");
                //         continue;
                //     }
                //     file_inode.write(offset, content.as_bytes());
                // };

                //
                // 目前写入的 content 没法换行，也就是读一串内容；
                // 如果要修改：循环读取 input，直到读到一个特殊字符
                //

                let mut offset;
                let next = input.next();

                if next.is_some() {
                    let arg = next.unwrap();
                    // 如果是 "a" 则追加 append
                    if arg.parse::<usize>().is_err() && arg == "-a" {
                        offset = file_inode.size();
                    } else {
                        offset = arg.parse::<usize>().unwrap();
                    }
                } else {
                    offset = 0;
                }

                println!("🐳 write: Please input content, end with newline EOF. 🐬");

                loop {
                    let mut content: String = String::new();
                    stdin().read_line(&mut content).unwrap();
                    if content == "EOF" || content == "EOF\n" {
                        // 让文件的最后一行不是空行
                        file_inode.write(offset - 1, "".as_bytes());
                        break;
                    }
                    file_inode.write(offset, content.as_bytes());
                    offset += content.len();
                }
            }

            // simple: get size of files
            "stat" => {
                let file_name = input.next();
                if file_name.is_none() {
                    println!("🦀 stat: Miss file name! 🦐");
                    continue;
                }
                let file_name = file_name.unwrap();
                let file_inode = curr_folder_inode.find(file_name);
                if file_inode.is_none() {
                    println!("🦀 stat: File not found! 🦐");
                    continue;
                }
                let file_inode = file_inode.unwrap();
                let size = file_inode.size();
                let (block_id, block_offset) = file_inode.inode_info();
                println!("🐳 The size of {} is {} B.", file_name, size);
                println!("🐳 The block_id of {}'s inode is {}.", file_name, block_id);
                println!(
                    "🐳 The block_offset of {}'s inode is {}.",
                    file_name, block_offset
                );
                println!("🦀🦀🦀🦀🦀🦀🦀\nThe following is the disK_inode info:");
                file_inode.dist_inode_info();
            }

            // 从 easy-fs 读取文件保存到 host 文件系统中
            "get" => {
                for file in curr_folder_inode.ls() {
                    // 从easy-fs中读取文件
                    println!("🐬 Get {} from easy-fs.", file);
                    let inode = curr_folder_inode.find(file.as_str()).unwrap();
                    let mut all_data: Vec<u8> = vec![0; inode.size() as usize];
                    inode.read(0, &mut all_data);
                    // 写入文件 保存到host文件系统中
                    let mut target_file = File::create(format!(
                        "{}{} {}",
                        target_path,
                        format!("{}", {
                            let fmt = "%Y-%m-%d %H:%M:%S"; // windows may be not support ":"
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
                    println!("🐳 Set {}{} to easy-fs.", src_path, file);
                    let mut host_file = File::open(format!("{}{}", src_path, file)).unwrap();
                    let mut all_data: Vec<u8> = Vec::new();
                    host_file.read_to_end(&mut all_data).unwrap();
                    // 创建文件
                    let inode = curr_folder_inode.create(file.as_str(), fs::DiskInodeType::File);
                    if inode.is_some() {
                        // 写入文件
                        let inode = inode.unwrap();
                        inode.write(0, all_data.as_slice());
                    }
                }
            }

            // 清空文件系统
            "fmt" => {
                println!("🐳 Worning!!!! 😱😱😱\n🐳 I have deleted all files in this folder! 🐬");
                let mut folder: Vec<Arc<Inode>> = Vec::new();
                let mut files: Vec<Arc<Inode>> = Vec::new(); // inclue folder
                drop(curr_folder_inode);
                curr_folder_inode = Arc::clone(&root_inode);

                // 递归遍历文件夹
                loop {
                    let all_files_name = curr_folder_inode.ls();
                    for file_name in all_files_name {
                        let inode = curr_folder_inode.find(file_name.as_str()).unwrap();
                        files.push(Arc::clone(&inode));
                        if inode.is_dir() {
                            folder.push(Arc::clone(&inode));
                        }
                    }
                    // 遍历所有文件夹
                    if folder.len() > 0 {
                        drop(curr_folder_inode);
                        curr_folder_inode = folder.pop().unwrap();
                    } else {
                        break;
                    }
                }

                // 清除所有文件 包括文件夹
                while files.len() > 0 {
                    let inode = files.pop().unwrap();
                    inode.clear();
                }

                // 对于根目录要特殊处理目录项
                let root_dir = Arc::clone(&root_inode);
                root_dir.clear();

                PATH.borrow_mut().clear();
                PATH.borrow_mut()
                    .push_str(&format!("❂ {}   ~\n╰─❯ ", USER));
            }

            "rm" => {
                let mut file = input.next();

                if file.is_none() {
                    println!("🦀 Please input file or folder name! 🦐");
                    continue;
                }

                loop {
                    if file.is_none() {
                        break;
                    }
                    let file_name = file.unwrap();
                    let file_inode = curr_folder_inode.find(file_name);
                    if file_inode.is_none() {
                        println!("🦀 rm: File not found! 🦐");
                        break;
                    }

                    let mut file_inode = file_inode.unwrap();

                    if file_inode.is_dir() {
                        let mut folder: Vec<Arc<Inode>> = Vec::new();
                        let mut files: Vec<Arc<Inode>> = Vec::new(); // inclue folder
                        let temp = Arc::clone(&file_inode);

                        // 递归遍历文件夹
                        loop {
                            let all_files_name = file_inode.ls();
                            for file_name in all_files_name {
                                let inode = file_inode.find(file_name.as_str()).unwrap();
                                files.push(Arc::clone(&inode));
                                if inode.is_dir() {
                                    folder.push(Arc::clone(&inode));
                                }
                            }
                            // 遍历所有文件夹
                            if folder.len() > 0 {
                                drop(file_inode);
                                file_inode = folder.pop().unwrap();
                            } else {
                                break;
                            }
                        }

                        // 清除所有文件 包括文件夹
                        while files.len() > 0 {
                            let inode = files.pop().unwrap();
                            inode.clear();
                        }

                        drop(file_inode);
                        file_inode = Arc::clone(&temp);
                    }

                    file_inode.clear();
                    file_inode.rm_dir_entry(file_name, Arc::clone(&curr_folder_inode));

                    file = input.next();
                }
            }

            "exit" => break,

            "help" => {
                println!("🐳 help: show helps.\n");
                println!("🐳 ls: list all files in current folder.\n");
                println!("🐳 cd: change current folder.\n");
                println!("🐳 cat: print file content.\n");
                println!("🐳 touch: create a file.\n");
                println!("🐳 mkdir: create a folder.\n");
                println!("🐳 stat: show file or folder stat.\n");
                println!("🐳 get: a test of fs, getting files to host form root directory.\n");
                println!("🐳 set: a test of fs, setting host files (src files of fs) to root directory.\n");
                println!("🐳 fmt: format easy-fs.\n");
                println!("🐳 exit: exit easy-fs.\n");

                println!("🐳 chname: change file or folder name.");
                println!("   🍡 usage: chname old_name new_name");
                println!("   🍡 note: the length of new_name is expected to be less than 27 ascii characters,");
                println!("          or no more than 9 unicode characters.");
                println!();

                println!("🐳 rm: remove files or folders.");
                println!("   🍡 usage: rm file1 folder2 file3 ...\n");

                println!("🐳 write: write content to file.");
                println!("   🍡 usage: write file_name (offset or \"-a\") content");
                println!("   🍡 offset: write content to file from offset.");
                println!("   🍡 -a: append content to file.");
                println!("   🍡 note: contents end with newline EOF.\n");

                println!("🐳 read: read content from file.");
                println!("   🍡 usage: read file_name (offset) (length)");
                println!("   🍡 offset: read content from file from offset.");
                println!("   🍡 length: read content length.");
                println!("   🍡 if offset and length are not set, read all content.\n");
            }
            _ => println!("🦀 Unknown command: {}! 🦐", cmd),
        }
    }

    Ok(())
}

fn update_path(target: &str) {
    // 如果 target 以 "/" 结尾，将 target 设置为 target 的子串
    let target = if target.ends_with('/') {
        &target[..target.len() - 1]
    } else {
        target
    };

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
