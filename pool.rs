
use std::cell::UnsafeCell;
use std::ptr::NonNull;
use std::alloc::{alloc, dealloc, Layout};

struct Block<T> {
    value: UnsafeCell<T>,
    counter: usize,
    index_in_page: usize,
}

use super::page::{BITFIELD_WIDTH, BLOCK_PER_PAGE};

struct Page<T> {
    bitfield: usize,
    blocks: [Block<T>; BLOCK_PER_PAGE]
}

pub struct PoolBox<T> {
    block: NonNull<Block<T>>,
    page: NonNull<Page<T>>
}

impl<T> PoolBox<T> {
    fn new(page: NonNull<Page<T>>, mut block: NonNull<Block<T>>) -> PoolBox<T> {
        // let counter = &mut unsafe { block.as_mut() }.counter;

        // assert!(*counter == 0, "PoolBox: Counter not zero {}", counter);

        // *counter = 1;

        PoolBox { block, page }
    }
}

impl<T> std::ops::Deref for PoolBox<T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.block.as_ref().value.get() }
    }
}

impl<T> std::ops::DerefMut for PoolBox<T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.block.as_ref().value.get() }
    }
}

/// Drop the PoolBox<T>
///
/// The value pointed by this PoolBox is also dropped
impl<T> Drop for PoolBox<T> {
    fn drop(&mut self) {
        let (page, block) = unsafe {
            (self.page.as_mut(), self.block.as_mut())
        };

        // See PoolBox<T>::new for why we touch the counter

        // assert!(block.counter == 1, "PoolBox: Counter != 1 on drop {}", block.counter);

        // block.counter = 0;

        unsafe {
            // Drop the inner value
            std::ptr::drop_in_place(block.value.get());
        }

        let index_in_page = block.index_in_page;

        page.bitfield |= 1 << index_in_page;

        // The bit dedicated to the Page is inversed (1 for used, 0 for free)
        if !page.bitfield == 1 << 63 {
            // We were the last block/arena referencing this page
            // Deallocate it
            page.deallocate();
        }
    }
}

impl<T> Page<T> {
    fn allocate() -> NonNull<Page<T>> {
        let layout = Layout::new::<Page<T>>();
        unsafe {
            let page = alloc(layout) as *const Page<T>;
            NonNull::from(&*page)
        }
    }

    pub fn deallocate(&mut self) {
        let layout = Layout::new::<Page<T>>();
        unsafe {
            dealloc(self as *mut Page<T> as *mut u8, layout);
        }
    }

    pub fn new() -> NonNull<Page<T>> {
        let mut page_ptr = Self::allocate();

        let page = unsafe { page_ptr.as_mut() };

        // We fill the bitfield with ones
        page.bitfield = !0;

        // initialize the blocks
        for (index, block) in page.blocks.iter_mut().enumerate() {
            block.index_in_page = index;
            block.counter = 0;
        }

        page_ptr
    }

    /// Search for a free [`Block`] in the [`Page`] and mark it as non-free
    ///
    /// If there is no free block, it returns None
    #[inline(never)]
    pub fn acquire_free_block(&mut self) -> Option<NonNull<Block<T>>> {
        let index_free = self.bitfield.trailing_zeros() as usize;

        if index_free == BLOCK_PER_PAGE {
            return None;
        }

        // We clear the bit of the free block to mark it as non free
        self.bitfield &= !(1 << index_free);

        Some(NonNull::from(&self.blocks[index_free]))
    }
}

pub struct Pool<T: Sized> {
    last_found: usize,
    pages: Vec<NonNull<Page<T>>>
}

impl<T: Sized> Pool<T> {
    pub fn new() -> Pool<T> {
        Self::with_capacity(1)
    }

    pub fn with_capacity(cap: usize) -> Pool<T> {
        let npages = ((cap.max(1) - 1) / BLOCK_PER_PAGE) + 1;

        let mut pages = Vec::with_capacity(npages);
        pages.resize_with(npages, Page::<T>::new);

        Pool { last_found: 0, pages }
    }

    fn alloc_new_page(&mut self) -> NonNull<Page<T>> {
        let len = self.pages.len();
        let new_len = len + len.min(900_000);

        self.pages.resize_with(new_len, Page::<T>::new);

        self.pages[len]
    }

    #[inline(never)]
    fn find_place(&mut self) -> (NonNull<Page<T>>, NonNull<Block<T>>) {
        let pages_len = self.pages.len();

        let mut last_found = self.last_found;

        if let Some(page) = self.pages.get_mut(last_found) {
            if let Some(block) = unsafe { page.as_mut() }.acquire_free_block() {
                // self.last_found += index;
                return (*page, block);
            };
        };

        last_found = last_found % pages_len;

        let (before, after) = self.pages
                                  .as_mut_slice()
                                  .split_at_mut(last_found);

        for (index, page) in after.iter_mut().chain(before).enumerate() {
            if let Some(block) = unsafe { page.as_mut() }.acquire_free_block() {
                if index != 0 {
                    self.last_found += index;
                }
                return (*page, block);
            };
        }

        let mut new_page = self.alloc_new_page();
        let node = unsafe { new_page.as_mut() }.acquire_free_block().unwrap();

        (new_page, node)
    }

    pub fn alloc(&mut self, value: T) -> PoolBox<T> {
        let (page, block) = self.find_place();

        unsafe {
            let ptr = block.as_ref().value.get();
            ptr.write(value);
        }

        PoolBox::new(page, block)
    }
}