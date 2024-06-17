pub(crate) struct Cipher {}

impl Cipher
{
    pub(crate) fn new(key: u64) -> Self
    {
        Self {}
    }

    pub(crate) fn encrypt_header(&self, block: &mut [u8])
    {
    }

    pub(crate) fn decrypt_header(&self, block: &mut [u8])
    {
    }

    pub(crate) fn encrypt_slot(&self, block: &mut [u8])
    {
    }

    pub(crate) fn decrypt_slot(&self, block: &mut [u8])
    {
    }
}
