use bufrw::BufReaderWriter;
use rand::seq::SliceRandom;
use std::io::Cursor;
use std::{
    io::{ Read, Seek, SeekFrom, Write},
};

struct FixedCSVFile<T> {
    field_sizes: Vec<usize>,
    buffer: Vec<u8>,
    stream: T,
}

impl<T> FixedCSVFile<T> {
    fn new(field_sizes: Vec<usize>, stream: T) -> Self {
        let len = field_sizes.iter().copied().max().unwrap();
        Self {
            field_sizes,
            buffer: vec![b' '; len],
            stream,
        }
    }

    fn record_size(&self) -> usize {
        self.field_sizes.iter().copied().sum::<usize>() + self.field_sizes.len()
    }
}

impl<T> FixedCSVFile<T>
where
    T: Write,
{
    fn write(&mut self, values: &[String]) -> std::io::Result<()> {
        assert_eq!(values.len(), self.field_sizes.len());
        for (i, (value, size)) in values
            .iter()
            .zip(self.field_sizes.iter().copied())
            .enumerate()
        {
            let bytes = value.as_bytes();
            let n = size.min(bytes.len());

            self.buffer[..n].copy_from_slice(&bytes[..n]);
            self.buffer[n..size].fill(b' ');

            self.stream.write_all(&self.buffer[..size])?;

            if i == self.field_sizes.len() - 1 {
                self.stream.write(&[b'\n'])?;
            } else {
                self.stream.write(&[b','])?;
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

impl<T> FixedCSVFile<T>
where
    T: Read,
{
    fn read(&mut self) -> std::io::Result<Vec<String>> {
        let mut values = Vec::with_capacity(self.field_sizes.len());
        for size in self.field_sizes.iter().copied() {
            self.stream.read_exact(&mut self.buffer[..size])?;
            let mut sep = [0u8];
            self.stream.read_exact(&mut sep)?;

            values.push(String::from_utf8(self.buffer.clone()).unwrap());
        }

        Ok(values)
    }
}

impl<T> FixedCSVFile<T>
where
    T: Seek,
{
    fn seek(&mut self, record_index: usize) -> std::io::Result<()> {
        let pos_in_bytes = self.record_size() * record_index;

        self.stream.seek(SeekFrom::Start(pos_in_bytes as u64))?;
        Ok(())
    }

    fn seek_relative(&mut self, n: i64) -> std::io::Result<()> {
        let n_in_bytes = self.record_size() as i64 * n;

        self.stream.seek(SeekFrom::Current(n_in_bytes))?;
        Ok(())
    }
}

struct FixedCsvTest {
    field_sizes: [usize; 2],
    records: Vec<[String; 2]>,
    expected_records: Vec<[String; 2]>,
    num_records: usize,
    num_random_seek_tests: usize,
    record_size: usize,
}

impl FixedCsvTest {
    fn new() -> Self {
        let field_sizes = [50; 2];
        let num_records = 10_000;
        assert_eq!(num_records % 2, 0);

        let num_random_seek_tests = 100;

        let records = vec![
            [String::from("Ulcerate"), String::from("Everything Is Fire")],
            [
                String::from("Insomnium"),
                String::from(" In the Halls of Awaiting"),
            ],
        ];

        let expected_records = vec![
            [
                format!("{:<50}", records[0][0]),
                format!("{:<50}", records[0][1]),
            ],
            [
                format!("{:<50}", records[1][0]),
                format!("{:<50}", records[1][1]),
            ],
        ];

        Self {
            field_sizes,
            records,
            expected_records,
            num_records,
            num_random_seek_tests,
            record_size: FixedCSVFile::new(field_sizes.to_vec(), Cursor::<Vec<u8>>::new(vec![]))
                .record_size(),
        }
    }

    fn write_base_data<T: Write>(&self, file: T) {
        let mut csv = FixedCSVFile::new(self.field_sizes.to_vec(), file);

        for i in 0..self.num_records {
            csv.write(&self.records[i % 2]).unwrap();
        }

        csv.flush().unwrap();
    }

    fn assert_records_are_in_order<T: Read>(&self, file: T) {
        let mut csv = FixedCSVFile::new(self.field_sizes.to_vec(), file);

        for i in 0..self.num_records {
            let values = csv.read().unwrap();
            assert_eq!(values.as_slice(), self.expected_records[i % 2].as_slice());
        }
    }

    fn assert_records_are_in_swapped_order<T: Read>(&self, file: T) {
        let mut csv = FixedCSVFile::new(self.field_sizes.to_vec(), file);

        for i in 0..self.num_records {
            let values = csv.read().unwrap();
            assert_eq!(
                values.as_slice(),
                self.expected_records[1 - (i % 2)].as_slice()
            );
        }
    }

    fn rewrite_in_swapped_order_using_seek_from_start<T: Read + Seek + Write>(
        &self,
        file: T,
        mut all_even_indices: Vec<usize>,
    ) {
        let mut csv = FixedCSVFile::new(self.field_sizes.to_vec(), file);


        while !all_even_indices.is_empty() {
            let index = all_even_indices.pop().unwrap();

            csv.seek(index).unwrap();

            let even_record = csv.read().unwrap();
            assert_eq!(even_record.as_slice(), self.expected_records[0].as_slice());
            let odd_record = csv.read().unwrap();
            assert_eq!(odd_record.as_slice(), self.expected_records[1].as_slice());

            csv.seek_relative(-2).unwrap();
            csv.write(&self.records[1]).unwrap();
            csv.write(&self.records[0]).unwrap();
            csv.seek_relative(-2).unwrap();
            let even_record = csv.read().unwrap();
            assert_eq!(even_record.as_slice(), self.expected_records[1].as_slice());
            let odd_record = csv.read().unwrap();
            assert_eq!(odd_record.as_slice(), self.expected_records[0].as_slice());
        }
        csv.flush().unwrap();
    }
}

#[test]
fn test_plain_read_write() {
    let tester = FixedCsvTest::new();

    let mut bufreadwrite = BufReaderWriter::new(Cursor::new(vec![]));

    let record_size = tester.record_size;
    let num_records = tester.num_records;

    // Write the base data to the file, using the bufr
    tester.write_base_data(&mut bufreadwrite);
    assert_eq!(bufreadwrite.inner().get_ref().len(), num_records * record_size);

    // Check the data is correct by reading directly the underlying file
    tester.assert_records_are_in_order(bufreadwrite.inner().get_ref().as_slice());

    // Then check the data is correct by reading via the bufrw
    bufreadwrite.seek(SeekFrom::Start(0)).unwrap();
    tester.assert_records_are_in_order(&mut bufreadwrite);
}

#[test]
fn test_rewrite_in_swapped_order_using_seek_from_start_increasing_order() {
    let tester = FixedCsvTest::new();

    let mut bufreadwrite = BufReaderWriter::new(Cursor::new(vec![]));

    let record_size = tester.record_size;
    let num_records = tester.num_records;

    // Write the base data to the file, using the bufr
    tester.write_base_data(&mut bufreadwrite);
    assert_eq!(bufreadwrite.inner().get_ref().len(), num_records * record_size);

    // Check the data is correct by reading directly the underlying file
    tester.assert_records_are_in_order(bufreadwrite.inner().get_ref().as_slice());

    // Then check the data is correct by reading via the bufrw
    bufreadwrite.seek(SeekFrom::Start(0)).unwrap();
    tester.assert_records_are_in_order(&mut bufreadwrite);

    // Test rewriting the data in swapped order
    // using indices in increasing order (0, 2, 4, 6, ...)
    bufreadwrite.seek(SeekFrom::Start(0)).unwrap();
    let all_even_indices = (0..tester.num_records)
        .filter(|i| i % 2 == 0)
        .collect::<Vec<_>>();
    tester.rewrite_in_swapped_order_using_seek_from_start(&mut bufreadwrite, all_even_indices);
    // Test the underlying data is correct
    tester.assert_records_are_in_swapped_order(bufreadwrite.inner().get_ref().as_slice());
    // Test reading via the bufrw is correct
    bufreadwrite.seek(SeekFrom::Start(0)).unwrap();
    tester.assert_records_are_in_swapped_order(&mut bufreadwrite);
}

#[test]
fn test_rewrite_in_swapped_order_using_seek_from_start_decreasing_order() {
    let tester = FixedCsvTest::new();

    let mut bufreadwrite = BufReaderWriter::new(Cursor::new(vec![]));

    let record_size = tester.record_size;
    let num_records = tester.num_records;

    // Write the base data to the file, using the bufr
    tester.write_base_data(&mut bufreadwrite);
    assert_eq!(bufreadwrite.inner().get_ref().len(), num_records * record_size);

    // Check the data is correct by reading directly the underlying file
    tester.assert_records_are_in_order(bufreadwrite.inner().get_ref().as_slice());

    // Then check the data is correct by reading via the bufrw
    bufreadwrite.seek(SeekFrom::Start(0)).unwrap();
    tester.assert_records_are_in_order(&mut bufreadwrite);

    // Test rewriting the data in swapped order
    // using indices in increasing order (0, 2, 4, 6, ...)
    bufreadwrite.seek(SeekFrom::Start(0)).unwrap();
    let mut all_even_indices = (0..tester.num_records)
        .filter(|i| i % 2 == 0)
        .collect::<Vec<_>>();
    all_even_indices.reverse();
    tester.rewrite_in_swapped_order_using_seek_from_start(&mut bufreadwrite, all_even_indices);
    // Test the underlying data is correct
    tester.assert_records_are_in_swapped_order(bufreadwrite.inner().get_ref().as_slice());
    // Test reading via the bufrw is correct
    bufreadwrite.seek(SeekFrom::Start(0)).unwrap();
    tester.assert_records_are_in_swapped_order(&mut bufreadwrite);
}

#[test]
fn test_rewrite_in_swapped_order_using_seek_from_start_random_order() {
    let tester = FixedCsvTest::new();

    for _ in 0..tester.num_random_seek_tests {
        let mut bufreadwrite = BufReaderWriter::new(Cursor::new(vec![]));

        let record_size = tester.record_size;
        let num_records = tester.num_records;

        // Write the base data to the file, using the bufr
        tester.write_base_data(&mut bufreadwrite);
        assert_eq!(bufreadwrite.inner().get_ref().len(), num_records * record_size);

        // Check the data is correct by reading directly the underlying file
        tester.assert_records_are_in_order(bufreadwrite.inner().get_ref().as_slice());

        // Then check the data is correct by reading via the bufrw
        bufreadwrite.seek(SeekFrom::Start(0)).unwrap();
        tester.assert_records_are_in_order(&mut bufreadwrite);

        // Test rewriting the data in swapped order
        // using indices in random order
        bufreadwrite.seek(SeekFrom::Start(0)).unwrap();
        let mut all_even_indices = (0..tester.num_records)
            .filter(|i| i % 2 == 0)
            .collect::<Vec<_>>();
        let mut rng = rand::rng();
        all_even_indices.shuffle(&mut rng);

        tester.rewrite_in_swapped_order_using_seek_from_start(&mut bufreadwrite, all_even_indices);
        // Test the underlying data is correct
        tester.assert_records_are_in_swapped_order(bufreadwrite.inner().get_ref().as_slice());
        // Test reading via the bufrw is correct
        bufreadwrite.seek(SeekFrom::Start(0)).unwrap();
        tester.assert_records_are_in_swapped_order(&mut bufreadwrite);
    }
}
