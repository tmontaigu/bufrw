use std::io::{Cursor, Read};
use profi::{print_on_exit, prof, prof_guard};
use rand::RngCore;

fn create_data_buffer() -> Cursor<Vec<u8>> {
    let mut rng = rand::rng();
    let mut bytes = vec![0; 500_000_000];
    rng.fill_bytes(&mut bytes);
    Cursor::new(bytes)
}

fn main() {
    let mut bytes = vec![0; 50];

    let total_num_bytes = 500_000_000;
    let num_writes = total_num_bytes / bytes.len();

    let mut cursor = create_data_buffer();

    // Prints the timings to stdout when the program exits
    // Always put at the top of the main function to ensure it's dropped last
    //
    // An implicit `main` guard is created to profile the whole application

    let mut buffer = vec![0; 8192].into_boxed_slice();

    // print_on_exit!();

    for _ in 0..10 {
        // prof!(iteration);
        {
            // prof_guard!("cursor.set_position");
            cursor.set_position(0);

        }
        let mut output = bufrw::BufReaderWriter::with_buffer(&mut cursor, buffer);
        for _ in 0..num_writes {
            output.read_exact(&mut bytes).unwrap();
        }

        buffer = output.into_parts().unwrap().1;
    }
}