#[cfg(test)]
mod wasm_parsers_tests {
    use log::debug;
    use num::checked_pow;
    use wabt::wat2wasm;
    use wasmbin::io::Encode;
    use wasmbin::Module;
    use wasmbin::sections::Kind;
    use wasmbin::visit::Visit;

    use crate::wasm_circuit::common::wasmbin_unlazify_with_opt;

    /// for dev only
    #[ignore]
    #[test]
    pub fn string_to_hex_bytes_test() {
        let strings = [
            "g1",
            "g2",
            "g3",
            "js",
            "global",
            "Hello, World",
            "none",
            "\0asm",
            "main",
            "memory",
            "table",
            "spectest",
            "env",
            "_evm_address",
            "_evm_balance",
            "_evm_some_long_name_func_some_long_name_func_some_long_name_func_some_long_name_func_some_long_name_func_some_long_name_func_some_long_name_func_some_long_name_func",
            "test",
            "global-i32",
        ];
        for str in strings {
            debug!("'{}' in hex {:x?} in decimal {:?}", str, str.to_string().as_bytes(), str.to_string().as_bytes());
        }
    }

    /// returns section len and quantity of leb bytes
    #[cfg(any(feature = "test", test))]
    fn compute_section_len(wasm_bytes: &Vec<u8>) -> (u32, u8) {
        let mut section_len: u32 = 0;
        const BASE_INDEX: usize = 1;
        let mut i = BASE_INDEX;
        loop {
            let byte = wasm_bytes[i];
            let mut byte_val: u32 = (byte & 0b1111111) as u32;
            byte_val = byte_val * checked_pow(0b10000000, i - BASE_INDEX).unwrap();
            section_len += byte_val;
            if byte & 0b10000000 == 0 { break }
            i += 1;
        }
        (section_len, (i - BASE_INDEX + 1) as u8)
    }

    #[test]
    pub fn test_print_parsed_file_contents() {
        let paths_to_files = [
            "./test_files/cc1.wat",
            "./test_files/cc2.wat",
            "./test_files/cc3.wat",
        ];
        for path_to_file in paths_to_files {
            let wat: Vec<u8> = std::fs::read(path_to_file).unwrap();
            debug!("SOURCE FILE: {}", path_to_file);
            debug!("source wat: {}", std::str::from_utf8(wat.as_slice()).unwrap());
            let wasm_binary = wat2wasm(wat.clone()).unwrap();

            let data = wat2wasm(&wat.clone()).unwrap();
            debug!("");
            debug!("data len: {}", data.len());
            debug!("data raw (hex): {:x?}", data);
            debug!("data raw (decimal): {:?}", data);

            let mut m = Module::decode_from(data.as_slice()).unwrap();
            for s in m.sections.iter_mut() {
                wasmbin_unlazify_with_opt(s, false).unwrap();
                debug!("---Kind::{:?}:", s.kind());
                let mut bytes = Vec::<u8>::new();
                s.encode(&mut bytes).unwrap();
                debug!("section len: {:?}", compute_section_len(&bytes));
                debug!("raw (hex): {:x?}", bytes);
                debug!("raw (decimal): {:?}", bytes);
                debug!("{:#?}", s);
                match s.kind() {
                    Kind::Type => {}
                    Kind::Code => {
                        for c1 in s.try_as_mut::<wasmbin::sections::payload::Code>().into_iter().enumerate() {
                            let code_contents = c1.1.try_contents().unwrap();
                            for c2 in code_contents.as_slice() {
                                let fb = c2.try_contents().unwrap();
                                for local in fb.locals.as_slice() {
                                    // local.
                                }
                                for exp in fb.expr.as_slice() {}
                            }
                        };
                        debug!("{:#?}", s);
                    },
                    Kind::Table => {},
                    Kind::Custom => {}
                    Kind::Element => {}
                    Kind::DataCount => {}
                    Kind::Import => {}
                    Kind::Function => {}
                    Kind::Memory => {}
                    Kind::Global => {}
                    Kind::Export => {}
                    Kind::Start => {}
                    Kind::Data => {}

                    _ => { debug!("UNPROCESSED/UNKNOWN section '{:?}'", s.kind()) }
                }
            }
            debug!("Found {} sections.", m.sections.len());
        }
    }
}