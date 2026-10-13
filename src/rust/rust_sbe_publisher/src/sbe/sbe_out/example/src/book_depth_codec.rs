use crate::*;

pub use encoder::*;
pub use decoder::*;

pub const SBE_BLOCK_LENGTH: u16 = 264;
pub const SBE_TEMPLATE_ID: u16 = 2;
pub const SBE_SCHEMA_ID: u16 = 1;
pub const SBE_SCHEMA_VERSION: u16 = 4;

pub mod encoder {
    use super::*;

    #[derive(Debug, Default)]
    pub struct BookDepthEncoder<'a> {
        buf: WriteBuf<'a>,
        initial_offset: usize,
        offset: usize,
        limit: usize,
    }

    impl<'a> Writer<'a> for BookDepthEncoder<'a> {
        #[inline]
        fn get_buf_mut(&mut self) -> &mut WriteBuf<'a> {
            &mut self.buf
        }
    }

    impl<'a> Encoder<'a> for BookDepthEncoder<'a> {
        #[inline]
        fn get_limit(&self) -> usize {
            self.limit
        }

        #[inline]
        fn set_limit(&mut self, limit: usize) {
            self.limit = limit;
        }
    }

    impl<'a> BookDepthEncoder<'a> {
        pub fn wrap(mut self, buf: WriteBuf<'a>, offset: usize) -> Self {
            let limit = offset + SBE_BLOCK_LENGTH as usize;
            self.buf = buf;
            self.initial_offset = offset;
            self.offset = offset;
            self.limit = limit;
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

        /// primitive array field 'symbol'
        /// - min value: 32
        /// - max value: 126
        /// - null value: 0
        /// - characterEncoding: US-ASCII
        /// - semanticType: null
        /// - encodedOffset: 0
        /// - encodedLength: 32
        /// - version: 0
        #[inline]
        pub fn symbol(&mut self, value: [u8; 32]) {
            let offset = self.offset;
            let buf = self.get_buf_mut();
            buf.put_u8_at(offset, value[0]);
            buf.put_u8_at(offset + 1, value[1]);
            buf.put_u8_at(offset + 2, value[2]);
            buf.put_u8_at(offset + 3, value[3]);
            buf.put_u8_at(offset + 4, value[4]);
            buf.put_u8_at(offset + 5, value[5]);
            buf.put_u8_at(offset + 6, value[6]);
            buf.put_u8_at(offset + 7, value[7]);
            buf.put_u8_at(offset + 8, value[8]);
            buf.put_u8_at(offset + 9, value[9]);
            buf.put_u8_at(offset + 10, value[10]);
            buf.put_u8_at(offset + 11, value[11]);
            buf.put_u8_at(offset + 12, value[12]);
            buf.put_u8_at(offset + 13, value[13]);
            buf.put_u8_at(offset + 14, value[14]);
            buf.put_u8_at(offset + 15, value[15]);
            buf.put_u8_at(offset + 16, value[16]);
            buf.put_u8_at(offset + 17, value[17]);
            buf.put_u8_at(offset + 18, value[18]);
            buf.put_u8_at(offset + 19, value[19]);
            buf.put_u8_at(offset + 20, value[20]);
            buf.put_u8_at(offset + 21, value[21]);
            buf.put_u8_at(offset + 22, value[22]);
            buf.put_u8_at(offset + 23, value[23]);
            buf.put_u8_at(offset + 24, value[24]);
            buf.put_u8_at(offset + 25, value[25]);
            buf.put_u8_at(offset + 26, value[26]);
            buf.put_u8_at(offset + 27, value[27]);
            buf.put_u8_at(offset + 28, value[28]);
            buf.put_u8_at(offset + 29, value[29]);
            buf.put_u8_at(offset + 30, value[30]);
            buf.put_u8_at(offset + 31, value[31]);
        }

        /// primitive field 'timestamp'
        /// - min value: 0
        /// - max value: -2
        /// - null value: -1
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 32
        /// - encodedLength: 8
        #[inline]
        pub fn timestamp(&mut self, value: u64) {
            let offset = self.offset + 32;
            self.get_buf_mut().put_u64_at(offset, value);
        }

        /// primitive field 'bidPrice0'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 40
        /// - encodedLength: 8
        #[inline]
        pub fn bid_price_0(&mut self, value: f64) {
            let offset = self.offset + 40;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'bidSize0'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 48
        /// - encodedLength: 8
        #[inline]
        pub fn bid_size_0(&mut self, value: f64) {
            let offset = self.offset + 48;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'bidPrice1'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 56
        /// - encodedLength: 8
        #[inline]
        pub fn bid_price_1(&mut self, value: f64) {
            let offset = self.offset + 56;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'bidSize1'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 64
        /// - encodedLength: 8
        #[inline]
        pub fn bid_size_1(&mut self, value: f64) {
            let offset = self.offset + 64;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'bidPrice2'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 72
        /// - encodedLength: 8
        #[inline]
        pub fn bid_price_2(&mut self, value: f64) {
            let offset = self.offset + 72;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'bidSize2'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 80
        /// - encodedLength: 8
        #[inline]
        pub fn bid_size_2(&mut self, value: f64) {
            let offset = self.offset + 80;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'bidPrice3'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 88
        /// - encodedLength: 8
        #[inline]
        pub fn bid_price_3(&mut self, value: f64) {
            let offset = self.offset + 88;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'bidSize3'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 96
        /// - encodedLength: 8
        #[inline]
        pub fn bid_size_3(&mut self, value: f64) {
            let offset = self.offset + 96;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'bidPrice4'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 104
        /// - encodedLength: 8
        #[inline]
        pub fn bid_price_4(&mut self, value: f64) {
            let offset = self.offset + 104;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'bidSize4'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 112
        /// - encodedLength: 8
        #[inline]
        pub fn bid_size_4(&mut self, value: f64) {
            let offset = self.offset + 112;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'bidPrice5'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 120
        /// - encodedLength: 8
        #[inline]
        pub fn bid_price_5(&mut self, value: f64) {
            let offset = self.offset + 120;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'bidSize5'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 128
        /// - encodedLength: 8
        #[inline]
        pub fn bid_size_5(&mut self, value: f64) {
            let offset = self.offset + 128;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'bidPrice6'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 136
        /// - encodedLength: 8
        #[inline]
        pub fn bid_price_6(&mut self, value: f64) {
            let offset = self.offset + 136;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'bidSize6'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 144
        /// - encodedLength: 8
        #[inline]
        pub fn bid_size_6(&mut self, value: f64) {
            let offset = self.offset + 144;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'askPrice0'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 152
        /// - encodedLength: 8
        #[inline]
        pub fn ask_price_0(&mut self, value: f64) {
            let offset = self.offset + 152;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'askSize0'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 160
        /// - encodedLength: 8
        #[inline]
        pub fn ask_size_0(&mut self, value: f64) {
            let offset = self.offset + 160;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'askPrice1'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 168
        /// - encodedLength: 8
        #[inline]
        pub fn ask_price_1(&mut self, value: f64) {
            let offset = self.offset + 168;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'askSize1'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 176
        /// - encodedLength: 8
        #[inline]
        pub fn ask_size_1(&mut self, value: f64) {
            let offset = self.offset + 176;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'askPrice2'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 184
        /// - encodedLength: 8
        #[inline]
        pub fn ask_price_2(&mut self, value: f64) {
            let offset = self.offset + 184;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'askSize2'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 192
        /// - encodedLength: 8
        #[inline]
        pub fn ask_size_2(&mut self, value: f64) {
            let offset = self.offset + 192;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'askPrice3'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 200
        /// - encodedLength: 8
        #[inline]
        pub fn ask_price_3(&mut self, value: f64) {
            let offset = self.offset + 200;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'askSize3'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 208
        /// - encodedLength: 8
        #[inline]
        pub fn ask_size_3(&mut self, value: f64) {
            let offset = self.offset + 208;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'askPrice4'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 216
        /// - encodedLength: 8
        #[inline]
        pub fn ask_price_4(&mut self, value: f64) {
            let offset = self.offset + 216;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'askSize4'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 224
        /// - encodedLength: 8
        #[inline]
        pub fn ask_size_4(&mut self, value: f64) {
            let offset = self.offset + 224;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'askPrice5'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 232
        /// - encodedLength: 8
        #[inline]
        pub fn ask_price_5(&mut self, value: f64) {
            let offset = self.offset + 232;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'askSize5'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 240
        /// - encodedLength: 8
        #[inline]
        pub fn ask_size_5(&mut self, value: f64) {
            let offset = self.offset + 240;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'askPrice6'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 248
        /// - encodedLength: 8
        #[inline]
        pub fn ask_price_6(&mut self, value: f64) {
            let offset = self.offset + 248;
            self.get_buf_mut().put_f64_at(offset, value);
        }

        /// primitive field 'askSize6'
        /// - min value: 4.9E-324
        /// - max value: 1.7976931348623157E308
        /// - null value: NaN
        /// - characterEncoding: null
        /// - semanticType: null
        /// - encodedOffset: 256
        /// - encodedLength: 8
        #[inline]
        pub fn ask_size_6(&mut self, value: f64) {
            let offset = self.offset + 256;
            self.get_buf_mut().put_f64_at(offset, value);
        }

    }

} // end encoder

pub mod decoder {
    use super::*;

    #[derive(Debug, Default)]
    pub struct BookDepthDecoder<'a> {
        buf: ReadBuf<'a>,
        initial_offset: usize,
        offset: usize,
        limit: usize,
        pub acting_block_length: u16,
        pub acting_version: u16,
    }

    impl<'a> Reader<'a> for BookDepthDecoder<'a> {
        #[inline]
        fn get_buf(&self) -> &ReadBuf<'a> {
            &self.buf
        }
    }

    impl<'a> Decoder<'a> for BookDepthDecoder<'a> {
        #[inline]
        fn get_limit(&self) -> usize {
            self.limit
        }

        #[inline]
        fn set_limit(&mut self, limit: usize) {
            self.limit = limit;
        }
    }

    impl<'a> BookDepthDecoder<'a> {
        pub fn wrap(
            mut self,
            buf: ReadBuf<'a>,
            offset: usize,
            acting_block_length: u16,
            acting_version: u16,
        ) -> Self {
            let limit = offset + acting_block_length as usize;
            self.buf = buf;
            self.initial_offset = offset;
            self.offset = offset;
            self.limit = limit;
            self.acting_block_length = acting_block_length;
            self.acting_version = acting_version;
            self
        }

        #[inline]
        pub fn encoded_length(&self) -> usize {
            self.limit - self.offset
        }

        pub fn header(self, mut header: MessageHeaderDecoder<ReadBuf<'a>>) -> Self {
            debug_assert_eq!(SBE_TEMPLATE_ID, header.template_id());
            let acting_block_length = header.block_length();
            let acting_version = header.version();

            self.wrap(
                header.parent().unwrap(),
                message_header_codec::ENCODED_LENGTH,
                acting_block_length,
                acting_version,
            )
        }

        #[inline]
        pub fn symbol(&self) -> [u8; 32] {
            let buf = self.get_buf();
            [
                buf.get_u8_at(self.offset),
                buf.get_u8_at(self.offset + 1),
                buf.get_u8_at(self.offset + 2),
                buf.get_u8_at(self.offset + 3),
                buf.get_u8_at(self.offset + 4),
                buf.get_u8_at(self.offset + 5),
                buf.get_u8_at(self.offset + 6),
                buf.get_u8_at(self.offset + 7),
                buf.get_u8_at(self.offset + 8),
                buf.get_u8_at(self.offset + 9),
                buf.get_u8_at(self.offset + 10),
                buf.get_u8_at(self.offset + 11),
                buf.get_u8_at(self.offset + 12),
                buf.get_u8_at(self.offset + 13),
                buf.get_u8_at(self.offset + 14),
                buf.get_u8_at(self.offset + 15),
                buf.get_u8_at(self.offset + 16),
                buf.get_u8_at(self.offset + 17),
                buf.get_u8_at(self.offset + 18),
                buf.get_u8_at(self.offset + 19),
                buf.get_u8_at(self.offset + 20),
                buf.get_u8_at(self.offset + 21),
                buf.get_u8_at(self.offset + 22),
                buf.get_u8_at(self.offset + 23),
                buf.get_u8_at(self.offset + 24),
                buf.get_u8_at(self.offset + 25),
                buf.get_u8_at(self.offset + 26),
                buf.get_u8_at(self.offset + 27),
                buf.get_u8_at(self.offset + 28),
                buf.get_u8_at(self.offset + 29),
                buf.get_u8_at(self.offset + 30),
                buf.get_u8_at(self.offset + 31),
            ]
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn timestamp(&self) -> u64 {
            self.get_buf().get_u64_at(self.offset + 32)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn bid_price_0(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 40)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn bid_size_0(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 48)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn bid_price_1(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 56)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn bid_size_1(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 64)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn bid_price_2(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 72)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn bid_size_2(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 80)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn bid_price_3(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 88)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn bid_size_3(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 96)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn bid_price_4(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 104)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn bid_size_4(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 112)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn bid_price_5(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 120)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn bid_size_5(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 128)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn bid_price_6(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 136)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn bid_size_6(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 144)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn ask_price_0(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 152)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn ask_size_0(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 160)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn ask_price_1(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 168)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn ask_size_1(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 176)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn ask_price_2(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 184)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn ask_size_2(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 192)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn ask_price_3(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 200)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn ask_size_3(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 208)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn ask_price_4(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 216)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn ask_size_4(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 224)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn ask_price_5(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 232)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn ask_size_5(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 240)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn ask_price_6(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 248)
        }

        /// primitive field - 'REQUIRED'
        #[inline]
        pub fn ask_size_6(&self) -> f64 {
            self.get_buf().get_f64_at(self.offset + 256)
        }

    }

} // end decoder

