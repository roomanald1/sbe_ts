use crate::*;

pub use decoder::*;
pub use encoder::*;

pub const SBE_BLOCK_LENGTH: u16 = 64;
pub const SBE_TEMPLATE_ID: u16 = 4;
pub const SBE_SCHEMA_ID: u16 = 1;
pub const SBE_SCHEMA_VERSION: u16 = 9;

pub mod encoder {
    use super::*;

    #[derive(Debug, Default)]
    pub struct EnrichedSymbolPriceEncoder<'a> {
        buf: WriteBuf<'a>,
        offset: usize,
        limit: usize,
    }

    impl<'a> Writer<'a> for EnrichedSymbolPriceEncoder<'a> {
        #[inline]
        fn get_buf_mut(&mut self) -> &mut WriteBuf<'a> {
            &mut self.buf
        }
    }

    impl<'a> Encoder<'a> for EnrichedSymbolPriceEncoder<'a> {
        #[inline]
        fn get_limit(&self) -> usize {
            self.limit
        }

        #[inline]
        fn set_limit(&mut self, limit: usize) {
            self.limit = limit;
        }
    }

    impl<'a> EnrichedSymbolPriceEncoder<'a> {
        pub fn wrap(mut self, buf: WriteBuf<'a>, offset: usize) -> Self {
            self.buf = buf;
            self.offset = offset;
            self.limit = offset + SBE_BLOCK_LENGTH as usize;
            self
        }

        #[inline]
        pub fn encoded_length(&self) -> usize {
            self.limit - self.offset
        }

        pub fn header(self, offset: usize) -> MessageHeaderEncoder<Self> {
            let mut header = MessageHeaderEncoder::default().wrap(self, offset);
            header.block_length(SBE_BLOCK_LENGTH);
            header.template_id(SBE_TEMPLATE_ID);
            header.schema_id(SBE_SCHEMA_ID);
            header.version(SBE_SCHEMA_VERSION);
            header
        }

        pub fn symbol(&mut self, value: [u8; 32]) {
            let offset = self.offset;
            for (index, byte) in value.iter().copied().enumerate() {
                self.get_buf_mut().put_u8_at(offset + index, byte);
            }
        }

        pub fn price(&mut self, value: f64) {
            let offset = self.offset + 32;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        pub fn timestamp(&mut self, value: u64) {
            let offset = self.offset + 40;
            self.get_buf_mut().put_u64_at(offset, value);
        }

        pub fn volatility_1m(&mut self, value: f64) {
            let offset = self.offset + 48;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        pub fn volatility_5m(&mut self, value: f64) {
            let offset = self.offset + 56;
            self.get_buf_mut().put_f64_at(offset, value);
        }
    }
}

pub mod decoder {
    use super::*;

    #[derive(Debug, Default)]
    pub struct EnrichedSymbolPriceDecoder<'a> {
        buf: ReadBuf<'a>,
        offset: usize,
        limit: usize,
    }

    impl<'a> Reader<'a> for EnrichedSymbolPriceDecoder<'a> {
        #[inline]
        fn get_buf(&self) -> &ReadBuf<'a> {
            &self.buf
        }
    }

    impl<'a> Decoder<'a> for EnrichedSymbolPriceDecoder<'a> {
        #[inline]
        fn get_limit(&self) -> usize {
            self.limit
        }

        #[inline]
        fn set_limit(&mut self, limit: usize) {
            self.limit = limit;
        }
    }

    impl<'a> EnrichedSymbolPriceDecoder<'a> {
        pub fn wrap(
            mut self,
            buf: ReadBuf<'a>,
            offset: usize,
            acting_block_length: u16,
            _acting_version: u16,
        ) -> Self {
            self.buf = buf;
            self.offset = offset;
            self.limit = offset + acting_block_length as usize;
            self
        }

        pub fn header(self, mut header: MessageHeaderDecoder<ReadBuf<'a>>) -> Self {
            debug_assert_eq!(SBE_TEMPLATE_ID, header.template_id());
            let block_length = header.block_length();
            let version = header.version();
            self.wrap(
                header.parent().unwrap(),
                message_header_codec::ENCODED_LENGTH,
                block_length,
                version,
            )
        }

        pub fn symbol(&self) -> [u8; 32] {
            std::array::from_fn(|index| self.get_buf().get_u8_at(self.offset + index))
        }

        pub fn price(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 32)
        }

        pub fn timestamp(&self) -> u64 {
            self.get_buf().get_u64_at(self.offset + 40)
        }

        pub fn volatility_1m(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 48)
        }

        pub fn volatility_5m(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 56)
        }
    }
}
