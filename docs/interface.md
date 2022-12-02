# Easy File System

#### EazyFileSystem::create

创建文件系统

- 初始化 `inode` 位图 (起始号，占用磁盘块数)
- (超级块位于第 0 块)
- 计算 `inode` 最大数量 -> 确定 `inode` 位图 与 `inode` 区域大小 -> 确定 数据位图 与 数据区域 大小
- 初始化 数据位图
- 初始化文件系统
- 清理 (既然是创建文件系统，第一次使用，需要将块设备的前 `total_blocks` 个块清零; 以块为单位，将块中的所有字节都设置为 0)
- 初始化超级块
- 为根目录 "/" 创建 `inode` (`alloc_inode`)
  [  `Bitmap::alloc` 在位图中寻找一个空闲 `bit` 分配，返回分配的 `inode_id` 编号。`BitmapBlock` 是一个 [u64; 64] 数组，找出最低位的 0(未分配)，修改为 1 (标记为分配) ]
- 初始化根目录:  通过 `inode_id` (`inode_id` = 0, 第一个 `inode`) 返回相应的 `block_id` 以及 对应块中的偏移
  [ `EazyFileSystem::get_disk_inode_pos` ]
- 同步块缓存 [`block_cache_sync_all`]



#### BlockCacheManager::get_block_cache

通过 `block_id` 查找缓存块

(目前 `BlockCacheManager` 以块号作为缓存块的区分，对于不同设备的同一个 block_id 会有冲突)

- 如果找到了就返回缓存块
- 否则需要从磁盘读数据到内存，执行缓存替换算法 (此处使用类 FIFO)
  - 考虑到从队列头开始查找，存在处在开头，但是仍在使用的缓存块，故需要从队头遍历到队尾找到第一个强引用计数恰好为 1 的块缓存并将其替换出去
  - 如果队列已满，且其中所有的块缓存都正在使用的情形直接 `panic` (简单的设计思路)

- 创建新的缓存块加入队列尾部
- 返回新创建的缓存块



#### BlockCache::read/modify

给出块缓存的偏移，根据具体的数据类型进行读/写

这个具体的类型实现类似于泛型：使用 闭包 将`BlockCahce::get_mut<T>` 与 `get_ref<T>` 进行封装

`get_mut<T>` 与 `get_ref<T>` 的返回值 (指针) 类型，恰好为闭包的传入参数类型

```rust
pub fn read<T, V>(&self, offset: usize, f: impl FnOnce(&T) -> V) -> V {
	f(self.get_ref(offset))
}

pub fn modify<T, V>(&mut self, offset: usize, f: impl FnOnce(&mut T) -> V) -> V {
	f(self.get_mut(offset))
}
```

```rust
/// 获取缓冲区中的位于偏移量 offset 的一个类型为 T 的磁盘上数据结构的不可变引用。
pub fn get_ref<T>(&self, offset: usize) -> &T
where
	T: Sized,
{
	let type_size = std::mem::size_of::<T>();
	// 确认 T 被整个包含在磁盘块及其缓冲区之内
	assert!(offset + type_size <= BLOCK_SIZE);
	let addr = self.addr_of_offset(offset);
	unsafe { &*(addr as *const T) }
}

/// get_mut 会获取磁盘上数据结构的可变引用，由此可以对数据结构进行修改。
/// 由于这些数据结构目前位于内存中的缓冲区中，
/// 我们需要将 BlockCache 的 modified 标记为 true 表示该缓冲区已经被修改，
/// 之后需要将数据写回磁盘块才能真正将修改同步到磁盘。
pub fn get_mut<T>(&mut self, offset: usize) -> &mut T
where
	T: Sized,
{
	let type_size = std::mem::size_of::<T>();
	assert!(offset + type_size <= BLOCK_SIZE);
	self.modified = true;
	let addr = self.addr_of_offset(offset);
	unsafe { &mut *(addr as *mut T) }
}
```

闭包 参数 捕获 `get_mut<T>` 与 `get_ref<T>` 的返回值 (对应类型的指针)

通过闭包 参数 的数据的类型，决定以何种形式 (何种类型的指针) 来访问内存，以此获取数据的引用进行读/修改

因此非常灵活 (这个类型可以是 `SuperBlock`, 可以是 `DataBlock`, 可以是 `BitmapBlock` 等等保存在缓存块中的数据)



#### DiskInode 与 Inode

`Inode` 可以通过 `read_disk_inode` 和 `modify_disk_inode` 来修改保存在磁盘上的 `DiskInode`

```rust
pub struct Inode {
    /// 位于哪个盘块
    block_id: usize,
    /// 盘块上的偏移
    block_offset: usize,
    fs: Arc<Mutex<EasyFileSystem>>,
    block_device: Arc<dyn BlockDevice>,
}
```

```rust
pub struct DiskInode {
    /// 文件/目录内容的字节数
    pub size: u32,
    /// 直接索引块
    pub direct: [u32; INODE_DIRECT_COUNT],
    /// 一级间接索引块
    pub indirect1: u32,
    /// 二级间接索引块
    pub indirect2: u32,
    /// 索引节点的类型 DiskInodeType ，目前仅支持文件 File 和目录 Directory 两种类型
    pub type_: DiskInodeType,
}
```

由于 `Inode` 知道自己的 `block_id` 与 `block_offset`，故可以通过

```rust
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
```

来修改 `DiskInode`



#### Inode::create

- 必须是在目录下创建文件
- 不可以重名
- 分配 `inode_id` 同时获取相应的 `block_id`，再初始化对应的 `DiskInode`

- 在对应的目录中插入目录项
- 同步块缓存 [`block_cache_sync_all`]



#### Inode::ls

通过 `read_disk_inode` 读取 `DiskInode` 大小，再除目录项大小可以获取有多少文件，再依次读取文件的名字即可 



#### Inode::find

通过文件名读取 DiskInode ，使用 `find_inode_id` 查找对应的 inode_id，再使用 `get_disk_inode_pos` 

通过 `inode_id` 返回相应的 `block_id` 以及对应块中的偏移，构造对应的 `Inode` 返回。



#### Inode::clear

- 对 `DiskInode` `clear_size`，回收 `block_id`
- `dealloc_data` 清空 `block_id` 对应的 `block`，再在 `data_bitmap` 中标记相应 `block_id` 未分配



#### DiskInode::get_block_id

从 `DiskInode` 中查到它自身用于保存文件内容的第 `block_id` 个数据块的块编号

传入参数 `inner_id` 是指 `DiskInode` 中的索引编号，用于定位是 直接/一级/二级 索引



#### DiskInode::read

将文件内容从 `offset` 字节开始的部分读到内存中的缓冲区 `buf` 中，并返回实际读到的字节数

- 通过 `offset` 定位到起始数据块 `start_block`
- 同过 `buf` 的长度定位到终止位置 `end`
- 通过  `get_block_id` 依次获取要读取的块，使用 `&mut` 切片依次读取到 `buf` 中



#### DiskInode::increase_size

参数 `new_size` 用于确认拓展后数据块数目

参数 `new_blocks` 保存了本次容量扩充所需块编号的向量，这些块都是由上层的磁盘块管理器负责分配的（通过 `EasyFileSystem::alloc_data` 获得）

- 根据自身大小以及修改后的大小确定当前数据块数目 `curr_blocks` 以及扩展后数据块数目`total_blocks`
- 依次分配索引
  - `new_blocks` 向量中保存了待分配的 数据索引块 与 数据块 的 `block_id`（数据索引块中保存数据块的 `block_id` )
  - 如果是到了一/二级索引，需要先分配一/二级索引的 `block_id` (给 `DiskInode::indirec1` \ `DiskInode::indirec2` 赋值)
  - 对于二级索引的分配，需要计算 当前二级索引的索引号、当前二级索引的一级子索引的索引号、目标二级索引的索引号、目标二级索引的一级子索引的索引号
  - 对于二级索引的一级子索引，填充到 以二级索引 `IndirectBlock` 数组元素的对应 `block_id` 的块中



#### DiskInode::clear_size

将大小清除为零并返回应释放的块，再将块内容清零

类似 `DiskInode::increase_size`，反向依次回收

最后将回收的所有块的编号保存在一个向量中返回给磁盘块管理器