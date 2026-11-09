//! Deployed over a WYRMLING contract by `deploy.mjs --step teardown`, so the account's state can be wiped and the
//! account deleted. Older protocols refuse DeleteAccount above 10,000 bytes of storage, code included, so this code
//! has no dependencies and stays under 1 KB.
//!
//! `clean` removes the storage keys in its raw input, each written as a little-endian u16 length and then the key
//! bytes. A missing key is skipped. Only the account itself may call it.
#![cfg_attr(target_arch = "wasm32", no_std)]

/// The keys of a `clean` input, in order, or None when the input is malformed.
pub fn keys(input: &[u8]) -> Option<Keys<'_>> {
    let mut rest = input;
    while !rest.is_empty() {
        let (len, tail) = rest.split_first_chunk::<2>()?;
        rest = tail.get(u16::from_le_bytes(*len) as usize..)?;
    }
    Some(Keys { rest: input })
}

pub struct Keys<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for Keys<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        let (len, tail) = self.rest.split_first_chunk::<2>()?;
        let (key, rest) = tail.split_at(u16::from_le_bytes(*len) as usize);
        self.rest = rest;
        Some(key)
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    #[link(wasm_import_module = "env")]
    extern "C" {
        fn input(register_id: u64);
        fn register_len(register_id: u64) -> u64;
        fn read_register(register_id: u64, ptr: u64);
        fn current_account_id(register_id: u64);
        fn predecessor_account_id(register_id: u64);
        fn storage_remove(key_len: u64, key_ptr: u64, register_id: u64) -> u64;
        fn panic_utf8(len: u64, ptr: u64) -> !;
    }

    const INPUT_MAX: usize = 1 << 17;
    static mut INPUT: [u8; INPUT_MAX] = [0; INPUT_MAX];

    fn fail(message: &str) -> ! {
        unsafe { panic_utf8(message.len() as u64, message.as_ptr() as u64) }
    }

    fn read(register: u64, buf: &mut [u8]) -> &[u8] {
        let len = unsafe { register_len(register) } as usize;
        if len > buf.len() {
            fail("input too large");
        }
        unsafe { read_register(register, buf.as_mut_ptr() as u64) };
        &buf[..len]
    }

    #[no_mangle]
    pub extern "C" fn clean() {
        let (mut me, mut caller) = ([0u8; 64], [0u8; 64]);
        unsafe {
            current_account_id(0);
            predecessor_account_id(1);
        }
        if read(0, &mut me) != read(1, &mut caller) {
            fail("only the account itself may clean");
        }
        unsafe { input(2) };
        let input = read(2, unsafe { &mut *core::ptr::addr_of_mut!(INPUT) });
        let keys = super::keys(input).unwrap_or_else(|| fail("bad keys"));
        for key in keys {
            unsafe { storage_remove(key.len() as u64, key.as_ptr() as u64, 3) };
        }
    }

    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        core::arch::wasm32::unreachable()
    }
}

#[cfg(test)]
mod tests {
    use super::keys;

    #[test]
    fn reads_each_length_prefixed_key() {
        let input = [1, 0, b's', 3, 0, b'l', 2, 0, 0, 0];
        let got: Vec<&[u8]> = keys(&input).unwrap().collect();
        assert_eq!(got, vec![&b"s"[..], &[b'l', 2, 0][..], &[][..]]);
    }

    #[test]
    fn refuses_a_key_cut_short() {
        assert!(keys(&[4, 0, 1, 2]).is_none());
        assert!(keys(&[1]).is_none());
    }

    #[test]
    fn an_empty_input_cleans_nothing() {
        assert_eq!(keys(&[]).unwrap().count(), 0);
    }
}
