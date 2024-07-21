use cipher::{
    array::Array,
    typenum::{U20, U8},
    BlockCipherDecrypt, BlockCipherEncrypt, KeyInit,
};
use rc5::RC5;

pub(crate) struct Cipher<const SIZE: usize>
{
    header_cipher: RC5<u16, U20, U8>,
    slot_cipher: RC5<u32, U20, U8>,
}

impl<const SIZE: usize> Cipher<SIZE>
{
    pub(crate) fn new(key: u64) -> Self
    {
        Self {
            header_cipher: RC5::new(key.to_ne_bytes().as_ref()),
            slot_cipher: RC5::new(key.to_ne_bytes().as_ref()),
        }
    }

    pub(crate) fn encrypt_header(&self, block: &mut [u8; 4])
    {
        self.header_cipher.encrypt_block(block.as_mut())
    }

    pub(crate) fn decrypt_header(&self, block: &mut [u8; 4])
    {
        self.header_cipher.decrypt_block(block.as_mut())
    }

    pub(crate) fn encrypt_slot(&self, block: &mut [u8; SIZE])
    {
        self.slot_cipher
            .encrypt_blocks(Array::cast_slice_from_core_mut(block.as_chunks_mut().0))
    }

    pub(crate) fn decrypt_slot(&self, block: &mut [u8; SIZE])
    {
        self.slot_cipher
            .decrypt_blocks(Array::cast_slice_from_core_mut(block.as_chunks_mut().0))
    }
}
