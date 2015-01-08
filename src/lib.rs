#![feature(slicing_syntax)]

extern crate lzf;
use std::str;
use lzf::decompress;
use std::io::MemReader;
use std::io;

mod helper;

mod version {
    pub const SUPPORTED_MINIMUM : u32 = 1;
    pub const SUPPORTED_MAXIMUM : u32 = 7;
}

mod constants {
    pub const RDB_6BITLEN : u8 = 0;
    pub const RDB_14BITLEN : u8 = 1;
    pub const RDB_ENCVAL : u8 = 3;
    pub const RDB_MAGIC : &'static str = "REDIS";
}

mod op_codes {
    pub const AUX : u8 = 250;
    pub const RESIZEDB : u8 = 251;
    pub const EXPIRETIME_MS : u8 = 252;
    pub const EXPIRETIME : u8 = 253;
    pub const SELECTDB   : u8 = 254;
    pub const EOF : u8 = 255;
}

mod types {
    pub const STRING : u8 = 0;
    pub const LIST : u8 = 1;
    pub const SET : u8 = 2;
    pub const ZSET : u8 = 3;
    pub const HASH : u8 = 4;
    pub const HASH_ZIPMAP : u8 = 9;
    pub const LIST_ZIPLIST : u8 = 10;
    pub const SET_INTSET : u8 = 11;
    pub const ZSET_ZIPLIST : u8 = 12;
    pub const HASH_ZIPLIST : u8 = 13;
}

mod encoding {
    pub const INT8 : u32 = 0;
    pub const INT16 : u32 = 1;
    pub const INT32 : u32 = 2;
    pub const LZF : u32 = 3;
}

#[derive(Show)]
pub enum DataType {
    String(Vec<u8>),
    Number(i64),
    ListOfTypes(Vec<DataType>),
    HashOfTypes(Vec<DataType>),
    SortedSetOfTypes(Vec<DataType>),
    Intset(Vec<i64>),
    List(Vec<Vec<u8>>),
    Set(Vec<Vec<u8>>),
    SortedSet(Vec<(f64,Vec<u8>)>),
    Hash(Vec<Vec<u8>>),
    Unknown
}


pub fn parse<R: Reader>(input: &mut R) {
    assert!(verify_magic(input));
    assert!(verify_version(input));

    let mut out = io::stdout();
    loop {
        let next_op = input.read_byte().unwrap();

        match next_op {
            op_codes::SELECTDB => {
                let db = read_length(input);
                println!("SELECTDB: {}", db);
            },
            op_codes::EOF => {
                println!("EOF");
                let chksum = input.read_to_end().unwrap();
                println!("checksum: {}", chksum);
                break ;
            },
            op_codes::EXPIRETIME_MS => {
                let expiretime_ms = input.read_le_u64().unwrap();
                println!("EXPIRETIME_MS: {}", expiretime_ms);
            },
            op_codes::EXPIRETIME => {
                let expiretime = input.read_be_u32().unwrap();
                println!("EXPIRETIME: {}", expiretime);
            },
            op_codes::RESIZEDB => {
                let db_size = read_length(input);
                let expires_size = read_length(input);
                println!("DB Size: {}, Expires Size: {}",
                         db_size, expires_size);
            },
            op_codes::AUX => {
                let auxkey = read_blob(input);
                let auxval = read_blob(input);

                formatter.aux_field(auxkey, auxval);
            },
            _ => {
                let key = read_blob(input);
                let _ = out.write(key[]);
                let _ = out.write_str(": ");
                let _ = out.flush();

                match read_type(next_op, input) {
                    DataType::String(t) => {
                        let _ = out.write(t[]);
                        let _ = out.write_str("\n");
                    },
                    DataType::Number(t) => { println!("{}", t) },
                    DataType::ListOfTypes(t) => { println!("{}", t) },
                    DataType::Intset(t) => { println!("{}", t) },
                    DataType::Hash(t) => {
                        for val in t.iter() {
                            let _ = out.write(val[]);
                            let _ = out.write_str(", ");
                        }
                        let _ = out.write_str("\n");
                    },
                    DataType::HashOfTypes(t) => { println!("{}", t) },
                    _ => {}
                }
            }
        }

    }

    return;

}

pub fn verify_magic<R: Reader>(input: &mut R) -> bool {
    let magic = input.read_exact(5).unwrap();

    // Meeeeeh.
    magic[0] == constants::RDB_MAGIC.as_bytes()[0] &&
        magic[1] == constants::RDB_MAGIC.as_bytes()[1] &&
        magic[2] == constants::RDB_MAGIC.as_bytes()[2] &&
        magic[3] == constants::RDB_MAGIC.as_bytes()[3] &&
        magic[4] == constants::RDB_MAGIC.as_bytes()[4]
}

pub fn verify_version<R: Reader>(input: &mut R) -> bool {
    let version = input.read_exact(4).unwrap();

    let version = (version[0]-48) as u32 * 1000 +
        (version[1]-48) as u32 * 100 +
        (version[2]-48) as u32 * 10 +
        (version[3]-48) as u32;

    version >= version::SUPPORTED_MINIMUM &&
        version <= version::SUPPORTED_MAXIMUM
}

fn read_linked_list<R: Reader>(input: &mut R) -> Vec<Vec<u8>> {
    let mut len = read_length(input);
    let mut list = vec![];

    while len > 0 {
        let blob = read_blob(input);
        list.push(blob);
        len -= 1;
    }

    list
}

fn read_sorted_set<R: Reader>(input: &mut R) -> Vec<(f64,Vec<u8>)> {
    let mut set = vec![];
    let mut set_items = read_length(input);

    while set_items > 0 {
        let val = read_blob(input);
        let score_length = input.read_byte().unwrap();
        let score = input.read_exact(score_length as uint).unwrap();
        let score = unsafe{str::from_utf8_unchecked(score[])}.
            parse::<f64>().unwrap();

        set.push((score, val));

        set_items -= 1;
    }

    set
}

fn read_hash<R: Reader>(input: &mut R) -> Vec<Vec<u8>> {
    let mut hash = vec![];
    let mut hash_items = read_length(input);
    hash_items = 2*hash_items;

    while hash_items > 0 {
        let val = read_blob(input);
        hash.push(val);

        hash_items -= 1;
    }

    hash
}

fn read_ziplist_entry<R: Reader>(input: &mut R) -> DataType {
    // 1. 1 or 5 bytes length of previous entry
    match input.read_byte().unwrap() {
        254 => {
            let _ = input.read_exact(4).unwrap();
        },
        _ => {}
    }

    let mut length : u64;
    let mut number_value : i64;

    // 2. Read flag or number value
    let flag = input.read_byte().unwrap();

    match (flag & 0xC0) >> 6 {
        0 => { length = (flag & 0x3F) as u64 },
        1 => {
            let next_byte = input.read_byte().unwrap();
            length = (((flag & 0x3F) as u64) << 8) | next_byte as u64;
        },
        2 => {
            length = input.read_be_u32().unwrap() as u64;
        },
        _ => {
            match (flag & 0xF0) >> 4 {
                0xC => { number_value = input.read_le_i16().unwrap() as i64 },
                0xD => { number_value = input.read_le_i32().unwrap() as i64 },
                0xE => { number_value = input.read_le_i64().unwrap() as i64 },
                0xF => {
                    match flag & 0xF {
                        0 => {
                            let bytes = input.read_exact(3).unwrap();
                            number_value = ((bytes[0] as i64) << 16) ^
                                ((bytes[1] as i64) << 8) ^
                                (bytes[2] as i64);
                        },
                        0xE => {
                            number_value = input.read_byte().unwrap() as i64 },
                            _ => { number_value = (flag & 0xF) as i64 - 1; }
                    }
                },
                _ => {
                    println!("Flag not handled: {}", flag);
                    return DataType::Number(-42);
                }

            }

            return DataType::Number(number_value)
        }
    }

    // 3. Read value
    let rawval = input.read_exact(length as uint).unwrap();
    DataType::String(rawval)
}

fn read_list_ziplist<R: Reader>(input: &mut R) -> Vec<DataType> {
    let ziplist = read_blob(input);

    let mut reader = MemReader::new(ziplist);

    let _zlbytes = reader.read_le_u32().unwrap();
    let _zltail = reader.read_le_u32().unwrap();
    let zllen = reader.read_le_u16().unwrap();
    let mut list = Vec::with_capacity(zllen as uint);

    for _ in range(0, zllen) {
        list.push(read_ziplist_entry(&mut reader));
    }

    assert!(reader.read_byte().unwrap() == 0xFF);

    list
}

fn read_zipmap_entry<R: Reader>(next_byte: u8, input: &mut R) -> Vec<u8> {
    let mut elem_len;
    match next_byte {
        253 => { elem_len = input.read_le_u32().unwrap() },
        254 | 255 => {
            panic!("Invalid length value in zipmap: {}", next_byte)
        },
        _ => { elem_len = next_byte as u32 }
    }

    input.read_exact(elem_len as uint).unwrap()
}

fn read_hash_zipmap<R: Reader>(input: &mut R) -> Vec<Vec<u8>> {
    let zipmap = read_blob(input);

    let mut reader = MemReader::new(zipmap);

    let zmlen = reader.read_byte().unwrap();

    let mut length;
    let mut hash;
    if zmlen <= 254 {
        length = zmlen as uint;
        hash = Vec::with_capacity(length);
    } else {
        length = -1;
        hash = Vec::with_capacity(255);
    }

    loop {
        let next_byte = reader.read_byte().unwrap();

        if next_byte == 0xFF {
            break; // End of list.
        }

        let key = read_zipmap_entry(next_byte, &mut reader);
        hash.push(key);

        let next_byte = reader.read_byte().unwrap();
        let _free = reader.read_byte().unwrap();
        let value = read_zipmap_entry(next_byte, &mut reader);
        hash.push(value);

        if length > 0 {
            length -= 1;
        }

        if length == 0 {
            assert!(reader.read_byte().unwrap() == 0xFF);
            break;
        }
    }

    hash
}

fn read_set_intset<R: Reader>(input: &mut R) -> Vec<i64> {
    let mut set = vec![];

    let intset = read_blob(input);

    let mut reader = MemReader::new(intset);
    let byte_size = reader.read_le_u32().unwrap();
    let intset_length = reader.read_le_u32().unwrap();

    for _ in range(0, intset_length) {
        let val = match byte_size {
            2 => reader.read_le_i16().unwrap() as i64,
            4 => reader.read_le_i32().unwrap() as i64,
            8 => reader.read_le_i64().unwrap(),
            _ => panic!("unhandled byte size in intset: {}", byte_size)
        };

        set.push(val);
    }

    set
}

fn read_type<R: Reader>(value_type: u8, input: &mut R) -> DataType {
    match value_type {
        types::STRING => {
            DataType::String(read_blob(input))
        },
        types::LIST => {
            DataType::List(read_linked_list(input))
        },
        types::SET => {
            DataType::Set(read_linked_list(input))
        },
        types::ZSET => {
            DataType::SortedSet(read_sorted_set(input))
        },
        types::HASH => {
            DataType::Hash(read_hash(input))
        },
        types::HASH_ZIPMAP => {
            DataType::Hash(read_hash_zipmap(input))
        },
        types::LIST_ZIPLIST => {
            DataType::ListOfTypes(read_list_ziplist(input))
        },
        types::SET_INTSET => {
            DataType::Intset(read_set_intset(input))
        },
        types::ZSET_ZIPLIST => {
            DataType::SortedSetOfTypes(read_list_ziplist(input))
        },
        types::HASH_ZIPLIST => {
            DataType::ListOfTypes(read_list_ziplist(input))
        },
        _ => { panic!("Value Type not implemented: {}", value_type) }
    }
}

pub fn read_length_with_encoding<R: Reader>(input: &mut R) -> (u32, bool) {
    let mut length;
    let mut is_encoded = false;

    let enc_type = input.read_byte().unwrap();

    match (enc_type & 0xC0) >> 6 {
        constants::RDB_ENCVAL => {
            is_encoded = true;
            length = (enc_type & 0x3F) as u32;
        },
        constants::RDB_6BITLEN => {
            length = (enc_type & 0x3F) as u32;
        },
        constants::RDB_14BITLEN => {
            let next_byte = input.read_byte().unwrap();
            length = (((enc_type & 0x3F) as u32) <<8) | next_byte as u32;
        },
        _ => {
            length = input.read_be_u32().unwrap();
        }
    }

    (length, is_encoded)
}

pub fn read_length<R: Reader>(input: &mut R) -> u32 {
    let (length, _) = read_length_with_encoding(input);
    length
}

pub fn read_blob<R: Reader>(input: &mut R) -> Vec<u8> {
    let (length, is_encoded) = read_length_with_encoding(input);

    if is_encoded {
        match length {
            encoding::INT8 => { helper::int_to_vec(input.read_i8().unwrap() as i32) },
            encoding::INT16 => { helper::int_to_vec(input.read_le_i16().unwrap() as i32) },
            encoding::INT32 => { helper::int_to_vec(input.read_le_i32().unwrap() as i32) },
            encoding::LZF => {
                let compressed_length = read_length(input);
                let real_length = read_length(input);
                let data = input.read_exact(compressed_length as uint).unwrap();
                lzf::decompress(data.as_slice(), real_length as uint).unwrap()
            },
            _ => { panic!("Unknown encoding: {}", length) }
        }
    } else {
        input.read_exact(length as uint).unwrap()
    }
}
