use std::{sync::{RwLock, RwLockReadGuard, Arc}, fs::File, io::{Seek, Write}};

use bit_vec::BitVec;

pub struct WriteMessage {
    index: u32,
    begin: u32,
    block: Vec<u8>,
}

impl WriteMessage {
    pub fn new(index: u32, begin: u32, block: &[u8]) -> Self {
        WriteMessage { index, begin, block: block.to_vec() }
    }

    pub const fn index(&self) -> u32 {
        self.index
    }

    pub const fn begin(&self) -> u32 {
        self.begin
    }

    pub const fn block(&self) -> &Vec<u8> {
        &self.block
    }
}

pub struct AtomicFile {
    file: RwLock<File>,
    bitfield: Arc<RwLock<BitVec>>,
    piece_size: u32,
}

impl AtomicFile {
    pub fn new(file: File, bitfield: BitVec, piece_size: u32) -> Self {
        Self {
            file: RwLock::new(file),
            bitfield: Arc::new(RwLock::new(bitfield)),
            piece_size: piece_size,
        }
    }

    /// doesn't write the piece, it tells the file that the piece should be written
    pub fn write_piece(&self, index: u32, begin: u32, block: &[u8]) -> std::io::Result<()> {
        // println!("writing piece. index: {}, begin: {}, block size: {}", index, begin, block.len());
        println!("test");
        // let mut file = self.file.write().unwrap();

        // let offset = (index as u64 * self.piece_size as u64) + begin as u64;

        // println!("test2");

        // file.seek(std::io::SeekFrom::Start(offset))?;
        // file.write_all(block)?;
        // file.flush()?;

        println!("begin: {}", begin);

        if block.len() < self.piece_size as usize || begin as u64 + self.piece_size as u64 >= self.file.read().unwrap().metadata().unwrap().len() {
            println!("piece {} completed", index);
            self.bitfield.write().unwrap().set(index as usize, true);
        }

        Ok(())
    }

    pub const fn bitfield(&self) -> &Arc<RwLock<BitVec>> {
        &self.bitfield
    }

    pub const fn piece_size(&self) -> u32 {
        self.piece_size
    }
}