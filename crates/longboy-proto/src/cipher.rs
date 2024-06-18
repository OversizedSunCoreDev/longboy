pub(crate) struct Cipher<const SIZE: usize> {}

impl<const SIZE: usize> Cipher<SIZE>
{
    pub(crate) fn new(key: u64) -> Self
    {
        Self {}
    }

    pub(crate) fn encrypt_header(&self, block: &mut [u8; 4])
    {
    }

    pub(crate) fn decrypt_header(&self, block: &mut [u8; 4])
    {
    }

    pub(crate) fn encrypt_slot(&self, block: &mut [u8; SIZE])
    {
    }

    pub(crate) fn decrypt_slot(&self, block: &mut [u8; SIZE])
    {
    }
}
