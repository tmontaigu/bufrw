//! buffered reading and writing over a single stream.
//!
//! This crate provides a single adapter that caches reads and writes on the same stream
//!
//! `BufReaderWriter` = `std::io::BufReader` + `std::io::BufWriter`
//!
//! Example
//!
//! ```rust
//! use bufrw::BufReaderWriter;
//! use std::io::{Cursor, Read, Seek, SeekFrom, Write};
//!
//! # fn main() -> std::io::Result<()> {
//! let inner = Cursor::new(String::from("Hello _____").into_bytes());
//! let mut rw = BufReaderWriter::new(inner);
//!
//! let mut s = String::new();
//! rw.read_to_string(&mut s)?;
//! assert_eq!(s, "Hello _____");
//!
//! // Replace the placeholder
//! rw.seek(SeekFrom::Current(-5))?;
//! rw.write_all(b"World")?;
//! rw.seek(SeekFrom::Start(0))?;
//!
//! s.clear();
//! rw.read_to_string(&mut s)?;
//! assert_eq!(s, "Hello World");
//!
//! let inner = rw.into_inner()?;
//! let underlying_bytes = inner.into_inner();
//! assert_eq!(underlying_bytes.as_slice(), "Hello World".as_bytes());
//!
//! # Ok::<_, std::io::Error>(())
//! # }
//! ```
use std::io::{Read, Seek, SeekFrom, Write};

/// Struct that adds buffering to any `T` that supports `Read`, `Write` and `Seek`
///
/// * Seeks do not invalidate the internal buffer if they don't need to
/// * Large (>= internal buffer's capacity) read/writes will bypass the buffer
pub struct BufReaderWriter<T: Write + Seek> {
    inner: T,
    pos: u64,
    // todo: rename to something more meaningful
    n: usize,
    buffer: Buffer,
}

impl<T> BufReaderWriter<T>
where
    T: Write + Seek,
{
    const DEFAULT_CAPACITY: usize = 8192;

    /// Creates a new BufReaderWriter from the input
    ///
    /// The buffer is allocated has the default capacity of `8KiB` (8192 bytes)
    pub fn new(inner: T) -> Self {
        Self::with_capacity(inner, Self::DEFAULT_CAPACITY)
    }

    /// Creates a new BufReaderWriter with the given capacity for the internal buffer
    pub fn with_capacity(inner: T, capacity: usize) -> Self {
        Self {
            inner,
            pos: 0,
            n: 0,
            buffer: Buffer::with_capacity(capacity),
        }
    }

    /// Returns the position in bytes in the data
    pub fn position(&self) -> u64 {
        self.start_position_in_source() + self.buffer.position() as u64
    }

    /// Returns the number of bytes the internal buffer can hold at once.
    pub fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    /// Returns a reference to the inner stream
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// Returns a mutable reference to the inner stream
    ///
    /// # Note
    ///
    /// The buffer may need to be flushed with [Self::flush_buffer] before
    ///
    /// Doing modification (read, write, seek) in the returned inner stream
    /// will cause problems unless carefully done.
    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Unwraps the BufReaderWriter, returning the inner stream
    ///
    /// This may flush the buffer before which could result in an error
    pub fn into_inner(mut self) -> std::io::Result<T> {
        if self.buffer.is_dirty {
            self.flush_buffer()?;
        }

        // Since `self` impl Drops we cannot simply deconstruct it
        let this = std::mem::ManuallyDrop::new(self);

        // SAFETY: double-drops are prevented by putting `this` in a ManuallyDrop that is never dropped

        let inner = unsafe { std::ptr::read(&this.inner) };

        Ok(inner)
    }

    /// Returns the current position in the source
    fn start_position_in_source(&self) -> u64 {
        self.pos - self.n as u64
    }

    /// Dump the buffer at the correct position
    ///
    /// Does not clear the buffer
    pub fn flush_buffer(&mut self) -> std::io::Result<()> {
        if self.n != 0 {
            let p = self.inner.seek(SeekFrom::Current(-(self.n as i64)))?;
            debug_assert_eq!(self.pos - self.n as u64, p);
            self.pos = p;
        }
        let n = self.buffer.dump(&mut self.inner)?;

        // This would mean we wrote fewer bytes than what we originally read
        debug_assert!(n >= self.n);

        self.pos += n as u64;
        self.n = n;
        Ok(())
    }
}

impl<T> Read for BufReaderWriter<T>
where
    T: Read + Write + Seek,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.buffer.get_read_command(buf) {
            ReadCommand::Read(n) => self.buffer.read(&mut buf[..n]),
            ReadCommand::FillRead { dump_before_fill } => {
                if dump_before_fill {
                    self.flush_buffer()?;
                    self.buffer.clear();
                    self.n = 0;
                }
                let n = self.buffer.fill_from(&mut self.inner)?;
                self.pos += n as u64;
                self.n = n;
                self.buffer.read(buf)
            }
            ReadCommand::ReadDirect { dump_before } => {
                if dump_before {
                    self.flush_buffer()?;
                    self.buffer.clear();
                    self.n = 0;
                }
                let n = self.inner.read(buf)?;
                self.pos += n as u64;
                Ok(n)
            }
        }
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> std::io::Result<()> {
        match self.buffer.get_read_exact_command(buf) {
            ReadExactCommand::Read => {
                self.buffer.read(buf)?;
            }
            ReadExactCommand::ReadFillRead { split, dump_before_fill } => {
                let (first, second) = buf.split_at_mut(split);
                self.buffer.read(first)?;
                if dump_before_fill {
                    self.flush_buffer()?;
                    self.buffer.clear();
                    self.n = 0;
                }
                let n = self.buffer.fill_from(&mut self.inner)?;
                self.pos += n as u64;
                self.n = n;
                self.buffer.read(second)?;
            }
            ReadExactCommand::FillRead { dump_before_fill } => {
                if dump_before_fill {
                    self.flush_buffer()?;
                    self.buffer.clear();
                    self.n = 0;
                }
                let n = self.buffer.fill_from(&mut self.inner)?;
                self.pos += n as u64;
                self.buffer.read(buf)?;
            }
            ReadExactCommand::ReadDirect { dump_before } => {
                if dump_before {
                    self.flush_buffer()?;
                    self.buffer.clear();
                    self.n = 0;
                }
                let n = self.inner.read(buf)?;
                self.pos += n as u64;
            }
            ReadExactCommand::ReadReadDirect { split, dump_before } => {
                let (first, second) = buf.split_at_mut(split);
                self.buffer.read(first)?;
                if dump_before {
                    self.flush_buffer()?;
                    self.buffer.clear();
                    self.n = 0;
                }
                let n= self.inner.read(second)?;
                self.pos += n as u64;
            }
        }
        Ok(())
    }
}

impl<T> Write for BufReaderWriter<T>
where
    T: Write + Seek,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.buffer.get_write_exact_command(buf) {
            WriteAllCommand::Write => self.buffer.write(buf),
            WriteAllCommand::WriteDumpWrite(n) => {
                let (first, second) = buf.split_at(n);
                self.buffer.write(first)?;
                self.flush_buffer()?;
                self.buffer.clear();
                self.n = 0;
                self.buffer.write(second)?;
                Ok(buf.len())
            }
            WriteAllCommand::DumpWriteDirect => {
                self.flush_buffer()?;
                self.buffer.clear();
                self.n = 0;
                self.inner.write(buf)
            }
            WriteAllCommand::WriteDirect => self.inner.write(buf),
        }
    }

    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        let _n = self.write(buf)?;
        debug_assert_eq!(_n, buf.len());
        Ok(())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flush_buffer()?;
        self.buffer.clear();
        self.n = 0;
        self.inner.flush()
    }
}

impl<T> Seek for BufReaderWriter<T>
where
    T: Write + Seek,
{
    /// Seek to an offset, in bytes,
    ///
    /// If the target position falls into the currently stored buffer,
    /// no seek in the underlying reader will happen.
    fn seek(&mut self, seek_from: SeekFrom) -> std::io::Result<u64> {
        match seek_from {
            SeekFrom::Start(pos) => {
                let in_mem_range = self.start_position_in_source()
                    ..self.start_position_in_source() + self.buffer.num_valid_bytes() as u64;
                if in_mem_range.contains(&pos) {
                    // We just need to adjust the position inside the buffer
                    self.buffer
                        .set_position(pos - self.start_position_in_source());
                    Ok(self.position())
                } else {
                    if self.buffer.is_dirty {
                        self.flush_buffer()?;
                    }
                    self.buffer.clear();
                    self.pos = self.inner.seek(SeekFrom::Start(pos))?;
                    self.n = 0;
                    Ok(self.position())
                }
            }
            SeekFrom::End(pos) => {
                if self.buffer.is_dirty {
                    self.flush_buffer()?;
                }
                self.buffer.clear();

                self.pos = self.inner.seek(SeekFrom::End(pos))?;
                self.n = 0;
                Ok(self.position())
            }
            SeekFrom::Current(direction) => {
                if direction == 0 {
                    // Shortcut as doing SeekFrom::Current(0) is common to get
                    // the position
                    Ok(self.position())
                } else if direction < 0 {
                    // Seeking backward by:
                    let abs_d = (-direction) as usize;

                    if abs_d > self.buffer.position() {
                        // Trying to seek to a place that is before what the buffer contains
                        if abs_d as u64 > self.position() {
                            return Err(std::io::Error::other("Seeking before start"));
                        }

                        if self.buffer.is_dirty {
                            self.flush_buffer()?;
                        }

                        self.pos = self.inner.seek(SeekFrom::Current(
                            direction - (self.n as i64 - self.buffer.position() as i64),
                        ))?;
                        self.buffer.clear();
                        self.n = 0;
                        Ok(self.pos)
                    } else {
                        // Trying to seek to a place that is within the buffer
                        self.buffer
                            .set_position((self.buffer.position() - abs_d) as u64);
                        Ok(self.position())
                    }
                } else {
                    // Seeking forward
                    let amount = direction as u64;

                    if amount >= self.buffer.num_readable_bytes_left() as u64 {
                        let saved_positon = self.position() as i64;
                        // Trying to seek to a place that is past what the buffer contains
                        if self.buffer.is_dirty {
                            self.flush_buffer()?;
                        }
                        self.buffer.clear();
                        self.n = 0;

                        let new_position = self.position() as i64;

                        self.pos = self
                            .inner
                            .seek(SeekFrom::Current(saved_positon - new_position + direction))?;
                        Ok(self.position())
                    } else {
                        // Trying to seek to a place that is within the buffer
                        self.buffer
                            .set_position(self.buffer.position() as u64 + amount);
                        Ok(self.position())
                    }
                }
            }
        }
    }

    fn stream_position(&mut self) -> std::io::Result<u64> {
        Ok(self.position())
    }
}

impl<T> Drop for BufReaderWriter<T>
where
    T: Write + Seek,
{
    fn drop(&mut self) {
        if self.buffer.is_dirty {
            let _ = self.flush();
        }
    }
}

/// After executing a command, all the requested bytes should have been written
/// unless an error occurred
enum WriteAllCommand {
    /// The buffer has enough capacity to store the data
    ///
    /// So, write to the buffer
    Write,
    /// The buffer does not have enough capacity to store the data
    ///
    /// Write to the buffer, then dump the buffer to the source
    /// and finally, write again to the buffer
    WriteDumpWrite(usize),
    /// Dump the buffer, then write directly to the source
    DumpWriteDirect,
    /// Write directly to the source
    WriteDirect,
}

/// After executing a command, not all bytes may have been read
enum ReadCommand {
    /// Read `n` bytes from the buffer
    Read(usize),
    /// Fill the buffer, then read all the bytes from the original request
    ///
    /// The buffer may need to be dumped before being refilled
    FillRead { dump_before_fill: bool },
    /// Read directly all the bytes from the original request from the source
    /// (skip the buffer)
    ///
    /// The buffer may need to be dumped before
    ReadDirect { dump_before: bool },
}

/// After executing a command, all bytes will be read
enum ReadExactCommand {
    /// The whole output can be filled bu reading from the buffer
    Read,
    /// Read from the buffer, re-fill the buffer, then read all the bytes from the original request
    ///
    /// The buffer may need to be dumped before being refilled
    ReadFillRead {
        split: usize,
        dump_before_fill: bool,
    },
    FillRead {
        dump_before_fill: bool,
    },
    /// Read directly all the bytes from the original request from the source
    /// (skip the buffer)
    ///
    /// The buffer may need to be dumped before
    ReadDirect {
        dump_before: bool,
    },
    /// Read from buffer, then finish reading from the source
    ReadReadDirect {
        split: usize,
        dump_before: bool,
    },
}

struct Buffer {
    data: Box<[u8]>,
    pos: usize,
    filled: usize,
    is_dirty: bool,
}

impl Buffer {
    fn with_capacity(capacity: usize) -> Self {
        let data = vec![0u8; capacity].into_boxed_slice();
        Self {
            data,
            pos: 0,
            filled: 0,
            is_dirty: false,
        }
    }

    #[inline]
    fn has_readable_bytes_left(&self) -> bool {
        self.pos != self.filled
    }

    #[inline]
    fn num_readable_bytes_left(&self) -> usize {
        self.filled - self.pos
    }

    #[inline]
    fn num_writable_bytes_left(&self) -> usize {
        self.capacity() - self.pos
    }

    #[inline]
    fn num_valid_bytes(&self) -> usize {
        self.filled
    }

    #[inline]
    fn capacity(&self) -> usize {
        self.data.len()
    }

    /// Fill the `self` from the `source`.
    ///
    /// This discards any data already present in `self`
    fn fill_from(&mut self, mut source: impl Read) -> std::io::Result<usize> {
        debug_assert!(!self.has_readable_bytes_left());
        let n = source.read(&mut self.data)?;
        self.filled = n;
        self.pos = 0;
        self.is_dirty = false;

        Ok(n)
    }

    #[inline]
    fn set_position(&mut self, pos: u64) {
        debug_assert!(pos < self.filled as u64);
        self.pos = pos.min(self.filled as u64) as usize;
    }

    #[inline]
    fn position(&self) -> usize {
        self.pos
    }

    fn dump(&mut self, mut dst: impl Write) -> std::io::Result<usize> {
        let n = self.filled;
        dst.write_all(&self.data[..n])?;
        Ok(n)
    }

    #[inline]
    fn clear(&mut self) {
        self.pos = 0;
        self.filled = 0;
        self.is_dirty = false;
    }

    #[inline]
    fn get_read_command(&self, buf: &[u8]) -> ReadCommand {
        if self.has_readable_bytes_left() {
            ReadCommand::Read(buf.len().min(self.num_readable_bytes_left()))
        } else if buf.len() >= self.capacity() {
            ReadCommand::ReadDirect {
                dump_before: self.is_dirty,
            }
        } else {
            ReadCommand::FillRead {
                dump_before_fill: self.is_dirty,
            }
        }
    }

    #[inline]
    fn get_read_exact_command(&self, buf: &[u8]) -> ReadExactCommand {
        if buf.len() >= self.capacity() {
            if self.has_readable_bytes_left() {
                ReadExactCommand::ReadReadDirect {
                    split: self.num_readable_bytes_left(),
                    dump_before: self.is_dirty,
                }
            } else {
                ReadExactCommand::ReadDirect {
                    dump_before: self.is_dirty,
                }
            }
        } else if self.num_readable_bytes_left() >= buf.len() {
            ReadExactCommand::Read
        } else if self.num_readable_bytes_left() < buf.len() {
            ReadExactCommand::ReadFillRead {
                split: self.num_readable_bytes_left(),
                dump_before_fill: self.is_dirty,
            }
        } else {
            debug_assert!(self.num_readable_bytes_left() == 0);
            ReadExactCommand::FillRead {
                dump_before_fill: self.is_dirty,
            }
        }
    }

    #[inline]
    fn get_write_exact_command(&self, buf: &[u8]) -> WriteAllCommand {
        if buf.len() >= self.capacity() {
            if self.is_dirty && self.num_valid_bytes() != 0 {
                WriteAllCommand::DumpWriteDirect
            } else {
                WriteAllCommand::WriteDirect
            }
        } else if self.num_writable_bytes_left() >= buf.len() {
            WriteAllCommand::Write
        } else {
            WriteAllCommand::WriteDumpWrite(self.num_writable_bytes_left())
        }
    }

    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.num_readable_bytes_left().min(buf.len());
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;

        debug_assert!(self.pos <= self.data.len());
        Ok(n)
    }

    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.num_writable_bytes_left().min(buf.len());
        if n == 0 {
            return Ok(0);
        }

        debug_assert!(self.pos + n <= self.capacity());
        if self.pos + n > self.filled {
            self.filled = self.pos + n;
        }
        self.data[self.pos..self.pos + n].copy_from_slice(&buf[..n]);
        self.pos += n;
        self.is_dirty = true;

        debug_assert!(self.pos <= self.filled);

        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::bool_assert_comparison)]
    use crate::BufReaderWriter;
    use rand::Rng;
    use std::io::{Cursor, Read, Seek, Write};

    #[test]
    fn test_seek_end_then_write() {
        let mut data = Cursor::new(vec![]);

        data.write_all(b"Yoshi").unwrap();
        data.set_position(0);

        let mut buf = BufReaderWriter::new(data);

        let n = buf.seek(std::io::SeekFrom::End(-3)).unwrap();
        assert_eq!(n, 2);

        buf.write_all(b"Yoshi").unwrap();
        assert!(buf.buffer.is_dirty);
        let n = buf.seek(std::io::SeekFrom::Start(0)).unwrap();
        assert_eq!(n, 0);

        let mut bytes = [0u8; 7];
        buf.read_exact(bytes.as_mut_slice()).unwrap();
        assert_eq!(&bytes, b"YoYoshi");
    }

    #[test]
    fn test_seek_current_negative_too_far() {
        let mut data = Cursor::new(vec![]);

        data.write_all(b"Yoshi").unwrap();
        data.set_position(0);

        let mut buf = BufReaderWriter::new(data);

        assert_eq!(buf.position(), 0);
        assert!(matches!(buf.stream_position(), Ok(0)));

        let result = buf.seek(std::io::SeekFrom::Current(-6));
        assert!(result.is_err());

        assert_eq!(buf.position(), 0);
        assert!(matches!(buf.stream_position(), Ok(0)));
    }

    #[test]
    fn test_seek_current_forward() {
        let mut rng = rand::rng();
        let mut cursor = Cursor::new(vec![]);
        let mut buf = BufReaderWriter::new(&mut cursor);
        let buf_capacity = buf.capacity();

        buf.inner.get_mut().resize(buf_capacity * 4, 0u8);
        for v in buf.inner.get_mut() {
            *v = rng.random();
        }

        let expected = buf.inner().get_ref().to_vec();

        let mut c = [0u8];
        buf.read_exact(&mut c).unwrap();
        assert_eq!(c[0], expected[0]);

        let n = buf.seek(std::io::SeekFrom::Current(1)).unwrap();
        assert_eq!(n, 2);

        buf.read_exact(&mut c).unwrap();
        assert_eq!(c[0], expected[2]);

        // Seek past buffer
        let n = buf
            .seek(std::io::SeekFrom::Current(buf_capacity as i64))
            .unwrap();
        assert_eq!(n, buf_capacity as u64 + 3);

        buf.read_exact(&mut c).unwrap();
        assert_eq!(c[0], expected[buf_capacity + 3])
    }

    #[test]
    fn test_seek_current_at_buffer_boundary() {
        let mut rng = rand::rng();
        let mut cursor = Cursor::new(vec![]);
        let mut buf = BufReaderWriter::new(&mut cursor);
        let buf_capacity = buf.capacity();

        // Fill the underlying source with some random data
        buf.inner
            .get_mut()
            .resize(buf_capacity + buf_capacity / 2, 0u8);
        for v in buf.inner.get_mut() {
            *v = rng.random();
        }

        // Clone it to have access to it without borrow problems
        let mut expected = buf.inner().get_ref().to_vec();

        let mut c = [0u8];
        buf.read_exact(&mut c).unwrap();
        assert_eq!(c[0], expected[0]);
        assert_eq!(buf.buffer.is_dirty, false);
        assert_eq!(buf.buffer.num_valid_bytes(), buf_capacity);
        assert_eq!(buf.buffer.num_readable_bytes_left(), buf_capacity - 1);
        assert_eq!(buf.buffer.num_writable_bytes_left(), buf_capacity - 1);
        assert_eq!(buf.position(), 1);

        let n = buf
            .seek(std::io::SeekFrom::Current(buf_capacity as i64 - 2))
            .unwrap();
        assert_eq!(n, buf_capacity as u64 - 1);
        assert_eq!(buf.buffer.is_dirty, false);
        assert_eq!(buf.buffer.num_valid_bytes(), buf_capacity);
        assert_eq!(buf.buffer.num_readable_bytes_left(), 1);
        assert_eq!(buf.buffer.num_writable_bytes_left(), 1);

        // This read_exact should trigger a refill as it crosses the buffer boundary
        let mut c = [0u8; 2];
        buf.read_exact(&mut c).unwrap();
        assert_eq!(&c, &expected[buf_capacity - 1..buf_capacity + 1]);
        assert_eq!(buf.buffer.is_dirty, false);
        assert_eq!(buf.buffer.num_valid_bytes(), buf_capacity / 2);
        assert_eq!(buf.buffer.num_readable_bytes_left(), buf_capacity / 2 - 1);
        assert_eq!(buf.buffer.num_writable_bytes_left(), buf_capacity - 1);

        // Seek back to before reading the 2 bytes
        let n = buf.seek(std::io::SeekFrom::Current(-2)).unwrap();
        assert_eq!(n, buf_capacity as u64 - 1);
        assert_eq!(buf.buffer.is_dirty, false);
        assert_eq!(buf.buffer.num_valid_bytes(), 0);
        assert_eq!(buf.buffer.num_readable_bytes_left(), 0);
        assert_eq!(buf.buffer.num_writable_bytes_left(), buf_capacity);

        let c2 = [c[0].wrapping_add(1), c[1].wrapping_add(1)];

        buf.write_all(&c2).unwrap();
        assert_eq!(buf.buffer.is_dirty, true);
        assert_eq!(buf.buffer.num_valid_bytes(), 2);
        assert_eq!(buf.buffer.num_readable_bytes_left(), 0);
        assert_eq!(buf.buffer.num_writable_bytes_left(), buf_capacity - 2);
        expected[n as usize] = c2[0];
        expected[n as usize + 1] = c2[1];

        // Seek back to before reading the 2 bytes
        let n = buf.seek(std::io::SeekFrom::Current(-2)).unwrap();
        assert_eq!(n, buf_capacity as u64 - 1);
        assert_eq!(buf.buffer.is_dirty, true);
        assert_eq!(buf.buffer.num_valid_bytes(), 2);
        assert_eq!(buf.buffer.num_readable_bytes_left(), 2);
        assert_eq!(buf.buffer.num_writable_bytes_left(), buf_capacity);

        let n = buf.seek(std::io::SeekFrom::Current(-2)).unwrap();
        assert_eq!(n, buf_capacity as u64 - 3);
        assert_eq!(buf.buffer.is_dirty, false); // a dump should have been done
        assert_eq!(buf.buffer.num_valid_bytes(), 0);
        assert_eq!(buf.buffer.num_readable_bytes_left(), 0);
        assert_eq!(buf.buffer.num_writable_bytes_left(), buf_capacity);

        let mut c = vec![0u8; 4];
        buf.read_exact(&mut c).unwrap();
        assert_eq!(&c, &expected[buf_capacity - 3..buf_capacity + 1]);
        assert_eq!(buf.buffer.is_dirty, false);
        assert_eq!(
            buf.buffer.num_valid_bytes(),
            expected.len() - (buf_capacity - 3)
        );
        assert_eq!(
            buf.buffer.num_readable_bytes_left(),
            buf.buffer.num_valid_bytes() - 4
        );
        assert_eq!(buf.buffer.num_writable_bytes_left(), buf_capacity - 4);

        buf.flush().unwrap();
        assert_eq!(buf.inner.get_ref(), expected.as_slice());
    }

    #[test]
    fn test_drop_flushes() {
        let mut cursor = Cursor::new(vec![]);
        let mut buf = BufReaderWriter::new(&mut cursor);

        assert_eq!(buf.position(), 0);
        assert!(matches!(buf.stream_position(), Ok(0)));

        assert_eq!(buf.buffer.is_dirty, false);
        assert_eq!(buf.buffer.num_readable_bytes_left(), 0);
        assert_eq!(buf.position(), 0);

        let data = b"Eco Dome Aldani";
        buf.write_all(data).unwrap();

        assert_eq!(buf.buffer.is_dirty, true);
        assert_eq!(buf.buffer.num_readable_bytes_left(), 0);
        assert_eq!(buf.position(), data.len() as u64);

        // Nothing was actually written yet
        assert_eq!(buf.inner().position(), 0);

        drop(buf);

        assert_eq!(cursor.position(), data.len() as u64);
        let s = String::from_utf8(cursor.into_inner()).unwrap();
        assert_eq!(s.as_bytes(), data);
    }

    #[test]
    fn write_more_than_buffer_capacity() {
        {
            // First, the simple case, where we never wrote not read anything
            // thus the buffer is empty

            let mut cursor = Cursor::new(vec![]);
            let mut buf = BufReaderWriter::new(&mut cursor);

            assert_eq!(buf.buffer.is_dirty, false);
            assert_eq!(buf.buffer.num_valid_bytes(), 0);

            let mut rng = rand::rng();
            let mut data = vec![0u8; buf.capacity()];
            for v in data.iter_mut() {
                *v = rng.random();
            }

            // Check that nothing was written in the buffer,
            // instead we wrote directly to the source
            buf.write_all(&data).unwrap();
            assert_eq!(buf.buffer.is_dirty, false);
            assert_eq!(buf.buffer.num_valid_bytes(), 0);
            assert_eq!(buf.inner().get_ref(), &data);
        }

        {
            // We wrote something before trying a write
            // with >= capacity

            let mut cursor = Cursor::new(vec![]);
            let mut buf = BufReaderWriter::new(&mut cursor);

            assert_eq!(buf.buffer.is_dirty, false);
            assert_eq!(buf.buffer.num_valid_bytes(), 0);

            let mut rng = rand::rng();
            let mut data = vec![0u8; buf.capacity() + 50];
            for v in data.iter_mut() {
                *v = rng.random();
            }

            let (first_write, second_write) = data.split_at_mut(50);

            buf.write_all(first_write).unwrap();

            assert_eq!(buf.buffer.is_dirty, true);
            assert_eq!(buf.buffer.num_valid_bytes(), 50);
            assert!(buf.inner().get_ref().is_empty());

            buf.write_all(second_write).unwrap();
            // The buffer has been dumped
            assert_eq!(buf.buffer.is_dirty, false);
            assert_eq!(buf.buffer.num_valid_bytes(), 0);
            assert_eq!(buf.inner().get_ref(), data.as_slice());
        }
    }

    #[test]
    fn read_more_than_buffer_capacity() {
        {
            // First, the simple case, where we never wrote not read anything
            // thus the buffer is empty

            let mut rng = rand::rng();
            let mut cursor = Cursor::new(vec![]);
            let mut buf = BufReaderWriter::new(&mut cursor);
            let buf_capacity = buf.capacity();
            let n = 4;

            buf.inner.get_mut().resize(buf_capacity * 4, 0u8);
            for v in buf.inner.get_mut() {
                *v = rng.random();
            }

            assert_eq!(buf.buffer.is_dirty, false);
            assert_eq!(buf.buffer.num_valid_bytes(), 0);

            let mut request = vec![0u8; buf.capacity()];
            for i in 0..n {
                buf.read_exact(&mut request).unwrap();
                assert_eq!(buf.buffer.is_dirty, false);
                assert_eq!(buf.buffer.num_valid_bytes(), 0);
                assert_eq!(
                    &buf.inner().get_ref()[i * buf_capacity..(i + 1) * buf_capacity],
                    &request
                );
            }
        }

        {
            // We read a small thing before trying a big read

            let mut rng = rand::rng();
            let mut cursor = Cursor::new(vec![]);
            let mut buf = BufReaderWriter::new(&mut cursor);
            let buf_capacity = buf.capacity();

            buf.inner.get_mut().resize((buf_capacity * 4) + 77, 0u8);
            for v in buf.inner.get_mut() {
                *v = rng.random();
            }

            assert_eq!(buf.buffer.is_dirty, false);
            assert_eq!(buf.buffer.num_valid_bytes(), 0);

            let mut first_request = vec![0u8; 104];
            buf.read_exact(&mut first_request).unwrap();
            assert_eq!(buf.buffer.is_dirty, false);
            assert_eq!(buf.buffer.num_valid_bytes(), buf_capacity);
            assert_eq!(
                buf.buffer.num_readable_bytes_left(),
                buf_capacity - first_request.len()
            );
            assert_eq!(&buf.inner().get_ref()[..104], &first_request);

            let cloned_data = buf.inner().get_ref().to_vec();
            let mut request = vec![0u8; buf.inner().get_ref().len() - first_request.len()];
            for (chunk_to_read, expected) in request
                .chunks_mut(buf_capacity)
                .zip(cloned_data[first_request.len()..].chunks(buf_capacity))
            {
                buf.read_exact(chunk_to_read).unwrap();
                assert_eq!(buf.buffer.is_dirty, false);
                assert_eq!(&chunk_to_read, &expected);
            }
        }

        {
            // We write a small thing before trying a big read

            let mut rng = rand::rng();
            let mut cursor = Cursor::new(vec![]);
            let mut buf = BufReaderWriter::new(&mut cursor);
            let buf_capacity = buf.capacity();

            buf.inner.get_mut().resize((buf_capacity * 4) + 77, 0u8);
            for v in buf.inner.get_mut() {
                *v = rng.random();
            }

            assert_eq!(buf.buffer.is_dirty, false);
            assert_eq!(buf.buffer.num_valid_bytes(), 0);

            let mut cloned_data = buf.inner().get_ref().to_vec();
            let mut data_to_write = vec![0u8; 77];
            for v in data_to_write.iter_mut() {
                *v = rng.random();
            }
            buf.write_all(&data_to_write).unwrap();
            assert_eq!(buf.buffer.is_dirty, true);
            cloned_data[..data_to_write.len()].copy_from_slice(&data_to_write);
            assert_eq!(buf.position(), data_to_write.len() as u64);

            let mut request = vec![0u8; cloned_data.len() - data_to_write.len()];
            for (chunk_to_read, expected) in request
                .chunks_mut(buf_capacity)
                .zip(cloned_data[data_to_write.len()..].chunks(buf_capacity))
            {
                buf.read_exact(chunk_to_read).unwrap();
                assert_eq!(buf.buffer.is_dirty, false);
                assert_eq!(&chunk_to_read, &expected);
            }
            assert_eq!(buf.inner.get_ref(), &cloned_data);
        }

        {
            // We read and write a small thing before trying a big read

            let mut rng = rand::rng();
            let mut cursor = Cursor::new(vec![]);
            let mut buf = BufReaderWriter::new(&mut cursor);
            let buf_capacity = buf.capacity();

            buf.inner.get_mut().resize((buf_capacity * 4) + 77, 0u8);
            for v in buf.inner.get_mut() {
                *v = rng.random();
            }

            assert_eq!(buf.buffer.is_dirty, false);
            assert_eq!(buf.buffer.num_valid_bytes(), 0);

            let mut first_request = vec![0u8; 104];
            buf.read_exact(&mut first_request).unwrap();
            assert_eq!(buf.buffer.is_dirty, false);
            assert_eq!(buf.buffer.num_valid_bytes(), buf_capacity);
            assert_eq!(
                buf.buffer.num_readable_bytes_left(),
                buf_capacity - first_request.len()
            );
            assert_eq!(
                &buf.inner().get_ref()[..first_request.len()],
                &first_request
            );
            assert_eq!(buf.position(), first_request.len() as u64);

            let mut cloned_data = buf.inner().get_ref().to_vec();
            let mut data_to_write = vec![0u8; 77];
            for v in data_to_write.iter_mut() {
                *v = rng.random();
            }
            buf.write_all(&data_to_write).unwrap();
            assert_eq!(buf.buffer.is_dirty, true);
            cloned_data[first_request.len()..data_to_write.len() + first_request.len()]
                .copy_from_slice(&data_to_write);
            assert_eq!(
                buf.position(),
                first_request.len() as u64 + data_to_write.len() as u64
            );

            let mut request =
                vec![0u8; cloned_data.len() - first_request.len() - data_to_write.len()];
            for (chunk_to_read, expected) in request
                .chunks_mut(buf_capacity)
                .zip(cloned_data[first_request.len() + data_to_write.len()..].chunks(buf_capacity))
            {
                buf.read_exact(chunk_to_read).unwrap();
                assert_eq!(buf.buffer.is_dirty, false);
                assert_eq!(&chunk_to_read, &expected);
            }
            assert_eq!(buf.inner.get_ref(), &cloned_data);
        }
    }
}
