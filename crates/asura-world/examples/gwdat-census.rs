//! Print what a `Gw.dat` archive says about itself, using `asura_world::gwdat`.
//!
//! ```text
//! cargo run -p asura-world --example gwdat-census -- C:\Users\Apo\Documents\GitHub\PreSearing\Gw.dat
//! ```
//!
//! The point is to have the archive's own numbers in one place while the decompressor is still to come:
//! the header, how many entries the file table holds, and the entries the map catalogue names. When
//! `FaDecompress` is ported, `--dump 0x345CC` from the reference's own `gwdat_scan.exe` becomes a byte
//! comparison against `stored()` output.

use asura_world::gwdat::GwDat;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: gwdat-census <Gw.dat> [file id in hex]");
        std::process::exit(2);
    };

    let mut archive = match GwDat::open(&path) {
        Ok(archive) => archive,
        Err(err) => {
            eprintln!("could not open {path}: {err}");
            std::process::exit(1);
        }
    };

    let header = *archive.header();
    println!("archive: {}", archive.path());
    println!(
        "header:  magic {:#010X}, header_size {}, block_size {}, mft at {} ({} bytes), flags {:#X}",
        header.magic,
        header.header_size,
        header.block_size,
        header.mft_offset,
        header.mft_size,
        header.flags
    );
    println!("entries: {}", archive.len());

    // The blobs the generated map catalogue names, from `tools/map-config.sql`.
    let named = [
        (
            "Lakeside County / Ascalon City / Ashford Abbey",
            0x1B97D_u32,
        ),
        ("The Catacombs", 0x5CEE6),
        ("Green Hills County / Barradin Estate", 0x5CF29),
        ("Wizard's Folly / Foibles Fair", 0x5CEE7),
        ("Regent Valley / Fort Ranik", 0x5CF2A),
        ("The Northlands", 0x8005_EA96),
        ("Kamadan, Jewel of Istan", 0x345CC),
    ];
    println!();
    println!("the map catalogue's blobs:");
    for (label, file_id) in named {
        match archive.entry(file_id) {
            Some(entry) => {
                let announced = archive
                    .announced_size(file_id)
                    .ok()
                    .flatten()
                    .map(|size| size.to_string())
                    .unwrap_or_else(|| "-".to_string());
                println!(
                    "  {file_id:#010X}  offset {:>13}  stored {:>10}  compressed {:>3}  decompressed {:>10}  {label}",
                    entry.offset, entry.size, entry.compressed, announced
                );
            }
            None => println!("  {file_id:#010X}  absent  {label}"),
        }
    }

    // A single id on the command line is decompressed and printed: the first bytes, its length, and a
    // printable prefix if it looks like a container. This is the shape a comparison with the reference's
    // `gwdat_scan Gw.dat --dump <id>` takes.
    if let Some(hex) = args.next() {
        let file_id = u32::from_str_radix(hex.trim_start_matches("0x"), 16).unwrap_or_else(|_| {
            eprintln!("{hex:?} is not a hex file id");
            std::process::exit(2);
        });
        match archive.file(file_id) {
            Ok(Some(bytes)) => {
                println!();
                println!(
                    "decompressed {file_id:#010X}: {} bytes, first 32: {}",
                    bytes.len(),
                    bytes
                        .iter()
                        .take(32)
                        .map(|byte| format!("{byte:02X}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
            Ok(None) => println!("\n{file_id:#010X} is not in the archive"),
            Err(err) => println!("\nreading {file_id:#010X} failed: {err}"),
        }
    }
}
