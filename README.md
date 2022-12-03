# fs-rs

An easy file system based on eazy-fs of rcore.

### Usage

if don't have rust environment, you can download rust by:

```bash
curl https://sh.rustup.rs -sSf | sh
```

then you can use `cargo` to build and run:

```bash
# for the first time
make create

# for the second time
make open
```

### Features:

- read: read a file randomly.
- write: write a file randomly.
- cd: change directory simply.
- ls: list files in current directory.
- mkdir: create a directory.
- touch: create a file.
- rm: remove a file or a directory.
- cat: print the content of a file.
- fmt: format the file system.
- chname: change the name of a file or a directory (a simple version of mv).
- set: a test for file system (copy files form host to easy-fs).
- get: a test for file system (copy files from easy-fs to host).

*maybe more in future:*

- mv: move a file or a directory.
- cp: copy a file or a directory.
- pwd: print the current directory.
- find: find a file or a directory.
- ln: create a link.
- stat: print the status of a file or a directory.



<img src="./docs/mixed_index_fs.png" alt="mixed_index_fs.png" width="80%" style="
  display: block;
  margin-left: auto;
  margin-right: auto;
  width: 50%;
">

<p style="text-align: right;">图片来源: xiaolincoding</p>
