extern crate integer_encoding;
extern crate crc;
extern crate byteorder;
extern crate failure;

use byteorder::{BigEndian, WriteBytesExt};
use crc::{crc32, Hasher32};
use integer_encoding::VarIntWriter;

use std::env;
use std::fs::File;
use std::io::{self, Read, Write};

type Result<T> = std::result::Result<T, failure::Error>;

fn main() {
  inner().unwrap();
}

fn inner() -> Result<()> {
  let file_names: Vec<String> = env::args().skip(1).collect();

  let mut stdout = io::stdout();

  // write magic bytes, version, and options
  stdout.write_all(&[0xFE, 0xED, 0xBE, 0xEF, 0x02])?;
  stdout.write_varint(file_names.len())?;

  for file_name in file_names {
    stdout.write_all(&[0x00])?;

    let name_bytes: Vec<u8> = file_name.bytes().collect();
    stdout.write_varint(name_bytes.len())?;
    stdout.write_all(&name_bytes)?;

    let mut f = File::open(&file_name)?;

    let mut crc = crc32::Digest::new(crc32::IEEE);
    let mut len = 0;

    let mut buf = [0; 512];

    loop {
      let read = f.read(&mut buf)?;
      if read == 0 {
        break;
      }
      len += read;
      crc.write(&buf[..read]);
    }

    // write crc
    stdout.write_u32::<BigEndian>(crc.sum32())?;
    // write len
    stdout.write_varint(len)?;
    // write contents
    let mut f = File::open(file_name)?;
    loop {
      let read = f.read(&mut buf)?;
      if read == 0 {
        break;
      }
      stdout.write_all(&buf[..read])?;
    }
  }

  Ok(())
}
