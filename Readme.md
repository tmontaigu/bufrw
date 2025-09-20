# bufrw

Buffered reading and writing over a single stream.

```toml
[dependencies]
bufrw = "0.1"
```


```rust
use bufrw::BufReaderWriter;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};

fn main() -> std::io::Result<()> {
    let inner = Cursor::new(b"Hello _____".to_vec());
    let mut rw = BufReaderWriter::new(inner);

    // Read
    let mut s = String::new();
    rw.read_to_string(&mut s)?;
    assert_eq!(s, "Hello _____");

    // Write after seeking back
    rw.seek(SeekFrom::Current(-5))?;
    rw.write_all(b"World")?;
    rw.seek(SeekFrom::Start(0))?;

    s.clear();
    rw.read_to_string(&mut s)?;
    assert_eq!(s, "Hello World");
    Ok(())
}
```