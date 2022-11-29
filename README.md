# fs-rs

based on eazy-fs of rcore

### create

创建虚拟磁盘：设置权限与大小

初始化文件系统

#### EazyFileSystem::create

- 初始化 inode 位图 (起始号，占用磁盘块数)
- (超级块位于第 0 块)
- 计算 inode 最大数量 -> 确定 inode 位图 与 inode 区域大小 -> 确定 数据位图 与 数据区域 大小

- 初始化 数据位图
- 初始化文件系统
- 清理 (既然是创建文件系统，第一次使用，需要将块设备的前 total_blocks 个块清零; 以块为单位，将块中的所有字节都设置为 0)
- 初始化超级块
- 为根目录 "/" 创建 inode (alloc_inode)
- 初始化根目录: 通过 inode_id (inode_id = 0, 第一个 inode) 返回相应的 block_id 以及 对应块中的偏移



#### Bitmap::alloc



#### BlockCacheManager::get_block_cache

通过 block_id 查找缓存块

(目前 BlockCacheManager 以块号作为缓存块的区分，对于不同设备的同一个 block_id 会有冲突)

- 如果找到了就返回缓存块
- 否则需要从磁盘读数据到内存，执行缓存替换算法 (此处使用类 FIFO)
  - 考虑到从队列头开始查找，存在处在开头，但是仍在使用的缓存块，故需要从队头遍历到队尾找到第一个强引用计数恰好为 1 的块缓存并将其替换出去
  - 如果队列已满，且其中所有的块缓存都正在使用的情形直接 panic (简单的设计思路)

- 创建新的缓存块加入队列尾部
- 返回新创建的缓存块



#### BlockCache::read/modify

给出块缓存的偏移，根据具体的数据类型进行读/写

这个具体的类型实现类似于泛型：使用 闭包 将BlockCahce::get_mut\<T\> 与 get_ref\<T\> 进行封装

get_mut\<T\> 与 get_ref\<T\> 的返回值 (指针) 类型，恰好为闭包的传入参数类型

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

闭包 参数 捕获 get_mut\<T\> 与 get_ref\<T\> 的返回值 (对应类型的指针)

通过闭包 参数 的数据的类型，决定以何种形式 (何种类型的指针) 来访问内存，以此获取数据的引用进行读/修改

因此非常灵活 (这个类型可以是 SuperBlock, 可以是 DataBlock, 可以是 BitmapBlock 等等保存在缓存块中的数据)