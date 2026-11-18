//! A small streaming ZIP writer (deflate, data descriptors) for `.xlsx`
//! export. No part is held in memory; parts and the archive stay under
//! 4 GiB (no zip64), which the exporter guarantees by starting new sheets.

use std::io::{self, Write};

use crate::Error;

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const DESCRIPTOR: u32 = 0x0807_4b50;
const EOCD: u32 = 0x0605_4b50;
/// 1980-01-01 00:00 in DOS time.
const DOS_DATE: u16 = 0x0021;

struct Counting<W> {
    inner: W,
    n: u64,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(b)?;
        self.n += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

struct Done {
    name: String,
    crc: u32,
    csize: u32,
    usize: u32,
    offset: u32,
}

pub struct ZipWriter<W: Write> {
    w: Counting<W>,
    done: Vec<Done>,
}

fn too_large() -> Error {
    Error::Invalid("the export is too large for one spreadsheet file".into())
}

fn small(v: u64) -> Result<u32, Error> {
    u32::try_from(v).map_err(|_| too_large())
}

impl<W: Write> ZipWriter<W> {
    pub fn new(w: W) -> ZipWriter<W> {
        ZipWriter {
            w: Counting { inner: w, n: 0 },
            done: Vec::new(),
        }
    }

    /// Start a part; write its bytes to the returned writer, then call
    /// [`Part::finish`].
    pub fn part(&mut self, name: &str) -> Result<Part<'_, W>, Error> {
        let offset = small(self.w.n)?;
        let w = &mut self.w;
        w.write_all(&LOCAL.to_le_bytes())?;
        // version 2.0, flags: data descriptor + UTF-8 names, deflate
        w.write_all(&[20, 0, 0x08, 0x08, 8, 0, 0, 0])?;
        w.write_all(&DOS_DATE.to_le_bytes())?;
        w.write_all(&[0; 12])?; // crc and sizes follow the data
        w.write_all(&(name.len() as u16).to_le_bytes())?;
        w.write_all(&[0, 0])?;
        w.write_all(name.as_bytes())?;
        let start = w.n;
        Ok(Part {
            enc: flate2::write::DeflateEncoder::new(
                PartSink { zip: self },
                flate2::Compression::fast(),
            ),
            crc: crc32fast::Hasher::new(),
            usize: 0,
            name: name.to_string(),
            offset,
            start,
        })
    }

    /// Start a part written from blocks compressed elsewhere (in parallel,
    /// see [`Block::new`]); call [`RawPart::finish`] after the last one.
    pub fn raw_part(&mut self, name: &str) -> Result<RawPart<'_, W>, Error> {
        let offset = small(self.w.n)?;
        let w = &mut self.w;
        w.write_all(&LOCAL.to_le_bytes())?;
        w.write_all(&[20, 0, 0x08, 0x08, 8, 0, 0, 0])?;
        w.write_all(&DOS_DATE.to_le_bytes())?;
        w.write_all(&[0; 12])?;
        w.write_all(&(name.len() as u16).to_le_bytes())?;
        w.write_all(&[0, 0])?;
        w.write_all(name.as_bytes())?;
        let start = w.n;
        Ok(RawPart {
            zip: self,
            crc: crc32fast::Hasher::new(),
            usize: 0,
            name: name.to_string(),
            offset,
            start,
        })
    }

    /// Write the directory and return the inner writer.
    pub fn finish(mut self) -> Result<W, Error> {
        let cd = small(self.w.n)?;
        if self.done.len() > 0xFFFF {
            return Err(too_large());
        }
        for d in &self.done {
            let w = &mut self.w;
            w.write_all(&CENTRAL.to_le_bytes())?;
            w.write_all(&[20, 0, 20, 0, 0x08, 0x08, 8, 0, 0, 0])?;
            w.write_all(&DOS_DATE.to_le_bytes())?;
            w.write_all(&d.crc.to_le_bytes())?;
            w.write_all(&d.csize.to_le_bytes())?;
            w.write_all(&d.usize.to_le_bytes())?;
            w.write_all(&(d.name.len() as u16).to_le_bytes())?;
            w.write_all(&[0; 12])?;
            w.write_all(&d.offset.to_le_bytes())?;
            w.write_all(d.name.as_bytes())?;
        }
        let size = small(self.w.n - u64::from(cd))?;
        let n = self.done.len() as u16;
        let w = &mut self.w;
        w.write_all(&EOCD.to_le_bytes())?;
        w.write_all(&[0; 4])?;
        w.write_all(&n.to_le_bytes())?;
        w.write_all(&n.to_le_bytes())?;
        w.write_all(&size.to_le_bytes())?;
        w.write_all(&cd.to_le_bytes())?;
        w.write_all(&[0, 0])?;
        w.flush()?;
        Ok(self.w.inner)
    }
}

struct PartSink<'a, W: Write> {
    zip: &'a mut ZipWriter<W>,
}

impl<W: Write> Write for PartSink<'_, W> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.zip.w.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.zip.w.flush()
    }
}

/// One part being written.
pub struct Part<'a, W: Write> {
    enc: flate2::write::DeflateEncoder<PartSink<'a, W>>,
    crc: crc32fast::Hasher,
    usize: u64,
    name: String,
    offset: u32,
    start: u64,
}

impl<W: Write> Part<'_, W> {
    /// Bytes written to this part so far (before compression).
    pub fn written(&self) -> u64 {
        self.usize
    }

    pub fn finish(self) -> Result<(), Error> {
        let sink = self.enc.finish()?;
        let zip = sink.zip;
        let csize = small(zip.w.n - self.start)?;
        let usize = small(self.usize)?;
        let crc = self.crc.finalize();
        zip.w.write_all(&DESCRIPTOR.to_le_bytes())?;
        zip.w.write_all(&crc.to_le_bytes())?;
        zip.w.write_all(&csize.to_le_bytes())?;
        zip.w.write_all(&usize.to_le_bytes())?;
        zip.done.push(Done {
            name: self.name,
            crc,
            csize,
            usize,
            offset: self.offset,
        });
        Ok(())
    }
}

impl<W: Write> Write for Part<'_, W> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let n = self.enc.write(b)?;
        self.crc.update(&b[..n]);
        self.usize += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.enc.flush()
    }
}

/// Some bytes of a part, compressed on their own: deflate blocks ending
/// in a sync flush, so blocks compressed apart join into one stream.
pub struct Block {
    bytes: Vec<u8>,
    crc: crc32fast::Hasher,
    len: u64,
}

impl Block {
    pub fn new(plain: &[u8]) -> Result<Block, Error> {
        let mut crc = crc32fast::Hasher::new();
        crc.update(plain);
        let mut enc = flate2::write::DeflateEncoder::new(
            Vec::with_capacity(plain.len() / 4 + 64),
            flate2::Compression::fast(),
        );
        enc.write_all(plain)?;
        // A sync flush: the blocks end on a byte, none marked last.
        enc.flush()?;
        let bytes = std::mem::take(enc.get_mut());
        Ok(Block {
            bytes,
            crc,
            len: plain.len() as u64,
        })
    }

    /// Bytes before compression.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// A part written from [`Block`]s, in order.
pub struct RawPart<'a, W: Write> {
    zip: &'a mut ZipWriter<W>,
    crc: crc32fast::Hasher,
    usize: u64,
    name: String,
    offset: u32,
    start: u64,
}

impl<W: Write> RawPart<'_, W> {
    pub fn write_block(&mut self, b: Block) -> Result<(), Error> {
        self.zip.w.write_all(&b.bytes)?;
        self.crc.combine(&b.crc);
        self.usize += b.len;
        Ok(())
    }

    /// Bytes written to this part so far (before compression).
    pub fn written(&self) -> u64 {
        self.usize
    }

    pub fn finish(self) -> Result<(), Error> {
        // The last block: empty, fixed codes, marked last.
        self.zip.w.write_all(&[0x03, 0x00])?;
        let csize = small(self.zip.w.n - self.start)?;
        let usize = small(self.usize)?;
        let crc = self.crc.finalize();
        let w = &mut self.zip.w;
        w.write_all(&DESCRIPTOR.to_le_bytes())?;
        w.write_all(&crc.to_le_bytes())?;
        w.write_all(&csize.to_le_bytes())?;
        w.write_all(&usize.to_le_bytes())?;
        self.zip.done.push(Done {
            name: self.name,
            crc,
            csize,
            usize,
            offset: self.offset,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zipread::Zip;

    #[test]
    fn round_trips_through_the_reader() {
        let mut z = ZipWriter::new(Vec::new());
        let mut p = z.part("a/b.xml").unwrap();
        p.write_all(b"<x>hello</x>").unwrap();
        p.finish().unwrap();
        let mut p = z.part("c.txt").unwrap();
        for _ in 0..10_000 {
            p.write_all(b"line of text\n").unwrap();
        }
        p.finish().unwrap();
        let bytes = z.finish().unwrap();
        let zip = Zip::open(&bytes).unwrap();
        assert_eq!(zip.read_string("a/b.xml", 100).unwrap(), "<x>hello</x>");
        assert_eq!(zip.read_string("c.txt", 1 << 20).unwrap().len(), 130_000);
    }

    #[test]
    fn blocks_compressed_apart_join() {
        let mut z = ZipWriter::new(Vec::new());
        let mut p = z.raw_part("x.txt").unwrap();
        let parts: Vec<String> = (0..20).map(|i| format!("block {i} ").repeat(1000 + i)).collect();
        for t in &parts {
            p.write_block(Block::new(t.as_bytes()).unwrap()).unwrap();
        }
        p.write_block(Block::new(b"").unwrap()).unwrap();
        p.finish().unwrap();
        let bytes = z.finish().unwrap();
        let zip = Zip::open(&bytes).unwrap();
        assert_eq!(zip.read_string("x.txt", 1 << 24).unwrap(), parts.concat());
    }
}
