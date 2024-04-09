use core::mem::MaybeUninit;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(i32)]
pub enum ReturnCode {
    Ok = 0,
    StreamEnd = 1,
    NeedDict = 2,
    ErrNo = -1,
    StreamError = -2,
    DataError = -3,
    MemError = -4,
    BufError = -5,
    VersionError = -6,
}

impl From<i32> for ReturnCode {
    fn from(value: i32) -> Self {
        use ReturnCode::*;

        match value {
            0 => Ok,
            1 => StreamEnd,
            2 => NeedDict,
            -1 => ErrNo,
            -2 => StreamError,
            -3 => DataError,
            -4 => MemError,
            -5 => BufError,
            -6 => VersionError,
            _ => panic!("invalid return code {value}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct InflateConfig {
    pub window_bits: i32,
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Method {
    #[default]
    Deflated = 8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Strategy {
    #[default]
    Default = 0,
    Filtered = 1,
    HuffmanOnly = 2,
    Rle = 3,
    Fixed = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeflateConfig {
    pub level: i32,
    pub method: Method,
    pub window_bits: i32,
    pub mem_level: i32,
    pub strategy: Strategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Flush {
    #[default]
    NoFlush = 0,
    PartialFlush = 1,
    SyncFlush = 2,
    FullFlush = 3,
    Finish = 4,
    Block = 5,
    Trees = 6,
}

trait ZlibImplementation {
    type Stream;

    const NAME: &'static str;

    fn inflate_init(strm: *mut Self::Stream, config: InflateConfig) -> ReturnCode;

    fn inflate(strm: &mut Self::Stream, flush: Flush) -> ReturnCode;

    fn inflate_end(strm: &mut Self::Stream) -> ReturnCode;

    fn deflate_init(strm: *mut Self::Stream, config: DeflateConfig) -> ReturnCode;

    fn deflate(strm: &mut Self::Stream, flush: Flush) -> ReturnCode;

    fn deflate_end(strm: &mut Self::Stream) -> ReturnCode;

    fn set_in(strm: &mut Self::Stream, input: &[u8]);

    fn set_out_raw<T>(strm: &mut Self::Stream, ptr: *const T, len: usize);

    fn set_out(strm: &mut Self::Stream, output: &[MaybeUninit<u8>]) {
        Self::set_out_raw(strm, output.as_ptr(), output.len())
    }

    fn avail_out_mut(strm: &mut Self::Stream) -> &mut core::ffi::c_uint;
    fn avail_in_mut(strm: &mut Self::Stream) -> &mut core::ffi::c_uint;

    fn total_out(strm: &Self::Stream) -> usize;
}

trait DeflateImplementation {
    const NAME: &'static str;

    fn uncompress_slice<'a>(
        output: &'a mut [MaybeUninit<u8>],
        input: &[u8],
        config: InflateConfig,
    ) -> (&'a mut [u8], ReturnCode);

    fn compress_slice<'a>(
        output: &'a mut [MaybeUninit<u8>],
        input: &[u8],
        config: DeflateConfig,
    ) -> (&'a mut [u8], ReturnCode);
}

impl<T: ZlibImplementation> DeflateImplementation for T {
    const NAME: &'static str = <T as ZlibImplementation>::NAME;

    fn uncompress_slice<'a>(
        output: &'a mut [MaybeUninit<u8>],
        input: &[u8],
        config: InflateConfig,
    ) -> (&'a mut [u8], ReturnCode) {
        let dest_len = output.len();
        let mut dest_len_ptr = 0;

        // z_uintmax_t len, left;
        let mut left;
        let dest;
        let buf: &mut [u8] = &mut [1]; /* for detection of incomplete stream when *destLen == 0 */

        let mut len = input.len() as u64;
        if dest_len != 0 {
            left = dest_len as u64;
            dest_len_ptr = 0;
            dest = output.as_mut_ptr();
        } else {
            left = 1;
            dest = buf.as_mut_ptr().cast();
        }

        let mut stream = MaybeUninit::zeroed();
        let err = Self::inflate_init(stream.as_mut_ptr(), config);
        let stream = unsafe { stream.assume_init_mut() };

        if err != ReturnCode::Ok {
            return (&mut [], ReturnCode::from(err));
        }

        Self::set_in(stream, input);
        Self::set_out(stream, output);

        Self::set_out_raw(stream, dest, 0);

        let err = loop {
            if *Self::avail_out_mut(stream) == 0 {
                *Self::avail_out_mut(stream) = Ord::min(left, u32::MAX as u64) as u32;
                left -= *Self::avail_out_mut(stream) as u64;
            }

            if *Self::avail_out_mut(stream) == 0 {
                *Self::avail_in_mut(stream) = Ord::min(len, u32::MAX as u64) as u32;
                len -= *Self::avail_in_mut(stream) as u64;
            }

            let err = Self::inflate(stream, Flush::NoFlush as _);
            let err = ReturnCode::from(err);

            if err != ReturnCode::Ok as _ {
                break err;
            }
        };

        if dest_len != 0 {
            dest_len_ptr = Self::total_out(stream);
        } else if Self::total_out(stream) != 0 && err == ReturnCode::BufError as _ {
            left = 1;
        }

        Self::inflate_end(stream);

        let ret = match err {
            ReturnCode::StreamEnd => ReturnCode::Ok,
            ReturnCode::NeedDict => ReturnCode::DataError,
            ReturnCode::BufError if (left + *Self::avail_out_mut(stream) as u64) != 0 => {
                ReturnCode::DataError
            }
            _ => err,
        };

        // SAFETY: we have now initialized these bytes
        let output_slice = unsafe {
            std::slice::from_raw_parts_mut(output.as_mut_ptr() as *mut u8, dest_len_ptr as usize)
        };

        (output_slice, ret)
    }

    fn compress_slice<'a>(
        output: &'a mut [MaybeUninit<u8>],
        input: &[u8],
        config: DeflateConfig,
    ) -> (&'a mut [u8], ReturnCode) {
        let mut stream = MaybeUninit::zeroed();
        let err = Self::deflate_init(stream.as_mut_ptr(), config);

        if err != ReturnCode::Ok {
            return (&mut [], ReturnCode::from(err));
        }

        let stream = unsafe { stream.assume_init_mut() };

        Self::set_in(stream, input);
        Self::set_out(stream, output);

        let max = core::ffi::c_uint::MAX as usize;

        let mut left = output.len();
        let mut source_len = input.len();

        loop {
            if *Self::avail_out_mut(stream) == 0 {
                *Self::avail_out_mut(stream) = Ord::min(left, max) as _;
                left -= *Self::avail_out_mut(stream) as usize;
            }

            if *Self::avail_in_mut(stream) == 0 {
                *Self::avail_in_mut(stream) = Ord::min(source_len, max) as _;
                source_len -= *Self::avail_in_mut(stream) as usize;
            }

            let flush = if source_len > 0 {
                Flush::NoFlush
            } else {
                Flush::Finish
            };

            let err = Self::deflate(stream, flush);

            if err != ReturnCode::Ok {
                break;
            }
        }

        let err = Self::deflate_end(stream);
        let return_code: ReturnCode = ReturnCode::from(err);
        // may DataError if there was insufficient output space
        assert_eq!(ReturnCode::Ok, return_code);

        // SAFETY: we have now initialized these bytes
        let output_slice = unsafe {
            std::slice::from_raw_parts_mut(output.as_mut_ptr() as *mut u8, Self::total_out(stream))
        };

        (output_slice, ReturnCode::Ok)
    }
}

struct ZlibOg;

impl ZlibImplementation for ZlibOg {
    type Stream = libz_sys::z_stream;

    const NAME: &'static str = "zlib-og";

    fn inflate_init(strm: *mut Self::Stream, config: InflateConfig) -> ReturnCode {
        ReturnCode::from(unsafe {
            libz_sys::inflateInit2_(
                strm,
                config.window_bits,
                "1.2.8\0".as_ptr().cast(),
                core::mem::size_of::<Self::Stream>() as _,
            )
        })
    }

    fn inflate(strm: &mut Self::Stream, flush: Flush) -> ReturnCode {
        ReturnCode::from(unsafe { libz_sys::inflate(strm, flush as _) })
    }

    fn inflate_end(strm: &mut Self::Stream) -> ReturnCode {
        ReturnCode::from(unsafe { libz_sys::inflateEnd(strm) })
    }

    fn deflate_init(strm: *mut Self::Stream, config: DeflateConfig) -> ReturnCode {
        ReturnCode::from(unsafe {
            libz_sys::deflateInit2_(
                strm,
                config.level,
                config.method as i32,
                config.window_bits,
                config.mem_level,
                config.strategy as i32,
                "1.2.8\0".as_ptr().cast(),
                core::mem::size_of::<Self::Stream>() as _,
            )
        })
    }

    fn deflate(strm: &mut Self::Stream, flush: Flush) -> ReturnCode {
        ReturnCode::from(unsafe { libz_sys::deflate(strm, flush as _) })
    }

    fn deflate_end(strm: &mut Self::Stream) -> ReturnCode {
        ReturnCode::from(unsafe { libz_sys::deflateEnd(strm) })
    }

    fn set_in(strm: &mut Self::Stream, input: &[u8]) {
        strm.avail_in = input.len() as _;
        strm.next_in = input.as_ptr() as *mut _;
    }

    fn set_out_raw<T>(strm: &mut Self::Stream, ptr: *const T, len: usize) {
        strm.avail_out = len as _;
        strm.next_out = ptr as *mut _;
    }

    fn avail_out_mut(strm: &mut Self::Stream) -> &mut core::ffi::c_uint {
        &mut strm.avail_out
    }

    fn avail_in_mut(strm: &mut Self::Stream) -> &mut core::ffi::c_uint {
        &mut strm.avail_in
    }

    fn total_out(strm: &Self::Stream) -> usize {
        strm.total_out as usize
    }
}

struct ZlibNg;

impl ZlibImplementation for ZlibNg {
    type Stream = libz_ng_sys::z_stream;

    const NAME: &'static str = "zlib-ng";

    fn inflate_init(strm: *mut Self::Stream, config: InflateConfig) -> ReturnCode {
        ReturnCode::from(unsafe {
            libz_ng_sys::inflateInit2_(
                strm,
                config.window_bits,
                "2.1.0.devel\0".as_ptr().cast(),
                core::mem::size_of::<Self::Stream>() as _,
            )
        })
    }

    fn inflate(strm: &mut Self::Stream, flush: Flush) -> ReturnCode {
        ReturnCode::from(unsafe { libz_ng_sys::inflate(strm, flush as _) })
    }

    fn inflate_end(strm: &mut Self::Stream) -> ReturnCode {
        ReturnCode::from(unsafe { libz_ng_sys::inflateEnd(strm) })
    }

    fn deflate_init(strm: *mut Self::Stream, config: DeflateConfig) -> ReturnCode {
        ReturnCode::from(unsafe {
            libz_ng_sys::deflateInit2_(
                strm,
                config.level,
                config.method as i32,
                config.window_bits,
                config.mem_level,
                config.strategy as i32,
                "2.1.0.devel\0".as_ptr().cast(),
                core::mem::size_of::<Self::Stream>() as _,
            )
        })
    }

    fn deflate(strm: &mut Self::Stream, flush: Flush) -> ReturnCode {
        ReturnCode::from(unsafe { libz_ng_sys::deflate(strm, flush as _) })
    }

    fn deflate_end(strm: &mut Self::Stream) -> ReturnCode {
        ReturnCode::from(unsafe { libz_ng_sys::deflateEnd(strm) })
    }

    fn set_in(strm: &mut Self::Stream, input: &[u8]) {
        strm.avail_in = input.len() as _;
        strm.next_in = input.as_ptr() as *mut _;
    }

    fn set_out_raw<T>(strm: &mut Self::Stream, ptr: *const T, len: usize) {
        strm.avail_out = len as _;
        strm.next_out = ptr as *mut _;
    }

    fn avail_out_mut(strm: &mut Self::Stream) -> &mut core::ffi::c_uint {
        &mut strm.avail_out
    }

    fn avail_in_mut(strm: &mut Self::Stream) -> &mut core::ffi::c_uint {
        &mut strm.avail_in
    }

    fn total_out(strm: &Self::Stream) -> usize {
        strm.total_out as usize
    }
}

struct ZlibRs;

impl ZlibImplementation for ZlibRs {
    type Stream = libz_rs_sys::z_stream;

    const NAME: &'static str = "zlib-rs";

    fn inflate_init(strm: *mut Self::Stream, config: InflateConfig) -> ReturnCode {
        ReturnCode::from(unsafe {
            libz_rs_sys::inflateInit2_(
                strm,
                config.window_bits,
                "1.2.8\0".as_ptr().cast(),
                core::mem::size_of::<Self::Stream>() as _,
            )
        })
    }

    fn inflate(strm: &mut Self::Stream, flush: Flush) -> ReturnCode {
        ReturnCode::from(unsafe { libz_rs_sys::inflate(strm, flush as _) })
    }

    fn inflate_end(strm: &mut Self::Stream) -> ReturnCode {
        ReturnCode::from(unsafe { libz_rs_sys::inflateEnd(strm) })
    }

    fn deflate_init(strm: *mut Self::Stream, config: DeflateConfig) -> ReturnCode {
        ReturnCode::from(unsafe {
            libz_rs_sys::deflateInit2_(
                strm,
                config.level,
                config.method as i32,
                config.window_bits,
                config.mem_level,
                config.strategy as i32,
                "1.2.8\0".as_ptr().cast(),
                core::mem::size_of::<Self::Stream>() as _,
            )
        })
    }

    fn deflate(strm: &mut Self::Stream, flush: Flush) -> ReturnCode {
        ReturnCode::from(unsafe { libz_rs_sys::deflate(strm, flush as _) })
    }

    fn deflate_end(strm: &mut Self::Stream) -> ReturnCode {
        ReturnCode::from(unsafe { libz_rs_sys::deflateEnd(strm) })
    }

    fn set_in(strm: &mut Self::Stream, input: &[u8]) {
        strm.avail_in = input.len() as _;
        strm.next_in = input.as_ptr() as *mut _;
    }

    fn set_out_raw<T>(strm: &mut Self::Stream, ptr: *const T, len: usize) {
        strm.avail_out = len as _;
        strm.next_out = ptr as *mut _;
    }

    fn avail_out_mut(strm: &mut Self::Stream) -> &mut core::ffi::c_uint {
        &mut strm.avail_out
    }

    fn avail_in_mut(strm: &mut Self::Stream) -> &mut core::ffi::c_uint {
        &mut strm.avail_in
    }

    fn total_out(strm: &Self::Stream) -> usize {
        strm.total_out as usize
    }
}

struct ZlibCloudflare;

impl ZlibImplementation for ZlibCloudflare {
    type Stream = cloudflare_zlib_sys::z_stream;

    const NAME: &'static str = "zlib-cloudflare";

    fn inflate_init(strm: *mut Self::Stream, config: InflateConfig) -> ReturnCode {
        ReturnCode::from(unsafe {
            cloudflare_zlib_sys::inflateInit2_(
                strm,
                config.window_bits,
                "1.2.8\0".as_ptr().cast(),
                core::mem::size_of::<Self::Stream>() as _,
            )
        })
    }

    fn inflate(strm: &mut Self::Stream, flush: Flush) -> ReturnCode {
        ReturnCode::from(unsafe { cloudflare_zlib_sys::inflate(strm, flush as _) })
    }

    fn inflate_end(strm: &mut Self::Stream) -> ReturnCode {
        ReturnCode::from(unsafe { cloudflare_zlib_sys::inflateEnd(strm) })
    }

    fn deflate_init(strm: *mut Self::Stream, config: DeflateConfig) -> ReturnCode {
        ReturnCode::from(unsafe {
            cloudflare_zlib_sys::deflateInit2_(
                strm,
                config.level,
                config.method as i32,
                config.window_bits,
                config.mem_level,
                config.strategy as i32,
                "1.2.8\0".as_ptr().cast(),
                core::mem::size_of::<Self::Stream>() as _,
            )
        })
    }

    fn deflate(strm: &mut Self::Stream, flush: Flush) -> ReturnCode {
        ReturnCode::from(unsafe { cloudflare_zlib_sys::deflate(strm, flush as _) })
    }

    fn deflate_end(strm: &mut Self::Stream) -> ReturnCode {
        ReturnCode::from(unsafe { cloudflare_zlib_sys::deflateEnd(strm) })
    }

    fn set_in(strm: &mut Self::Stream, input: &[u8]) {
        strm.avail_in = input.len() as _;
        strm.next_in = input.as_ptr() as *mut _;
    }

    fn set_out_raw<T>(strm: &mut Self::Stream, ptr: *const T, len: usize) {
        strm.avail_out = len as _;
        strm.next_out = ptr as *mut _;
    }

    fn avail_out_mut(strm: &mut Self::Stream) -> &mut core::ffi::c_uint {
        &mut strm.avail_out
    }

    fn avail_in_mut(strm: &mut Self::Stream) -> &mut core::ffi::c_uint {
        &mut strm.avail_in
    }

    fn total_out(strm: &Self::Stream) -> usize {
        strm.total_out as usize
    }
}

struct MinizOxide;

impl DeflateImplementation for MinizOxide {
    const NAME: &'static str = "miniz-oxide";

    fn uncompress_slice<'a>(
        output: &'a mut [MaybeUninit<u8>],
        input: &[u8],
        _config: InflateConfig,
    ) -> (&'a mut [u8], ReturnCode) {
        let flags = miniz_oxide::inflate::core::inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER
            | miniz_oxide::inflate::core::inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;

        let mut output = unsafe {
            core::slice::from_raw_parts_mut(output.as_mut_ptr().cast::<u8>(), output.len())
        };

        let mut decomp = Box::<miniz_oxide::inflate::core::DecompressorOxide>::default();

        let mut out_pos = 0;
        loop {
            // Wrap the whole output slice so we know we have enough of the
            // decompressed data for matches.
            let (status, _in_consumed, out_consumed) =
                miniz_oxide::inflate::core::decompress(&mut decomp, input, output, out_pos, flags);
            out_pos += out_consumed;

            match status {
                miniz_oxide::inflate::TINFLStatus::Done => {
                    output = &mut output[..out_pos];
                    return (output, ReturnCode::Ok);
                }

                miniz_oxide::inflate::TINFLStatus::HasMoreOutput => {
                    unreachable!()
                }

                _ => unreachable!(),
            }
        }
    }

    fn compress_slice<'a>(
        output: &'a mut [MaybeUninit<u8>],
        mut input: &[u8],
        config: DeflateConfig,
    ) -> (&'a mut [u8], ReturnCode) {
        let mut output = unsafe {
            core::slice::from_raw_parts_mut(output.as_mut_ptr().cast::<u8>(), output.len())
        };

        // The comp flags function sets the zlib flag if the window_bits parameter is > 0.
        let flags = miniz_oxide::deflate::core::create_comp_flags_from_zip_params(
            config.level.into(),
            config.window_bits as i32,
            config.strategy as i32,
        );
        let mut compressor = miniz_oxide::deflate::core::CompressorOxide::new(flags);

        let mut out_pos = 0;
        loop {
            let (status, bytes_in, bytes_out) = miniz_oxide::deflate::core::compress(
                &mut compressor,
                input,
                &mut output[out_pos..],
                miniz_oxide::deflate::core::TDEFLFlush::Finish,
            );
            out_pos += bytes_out;

            match status {
                miniz_oxide::deflate::core::TDEFLStatus::Done => {
                    output = &mut output[..out_pos];
                    break;
                }
                miniz_oxide::deflate::core::TDEFLStatus::Okay if bytes_in <= input.len() => {
                    input = &input[bytes_in..];

                    if true {
                        unreachable!("we should provide enough space");
                    }
                }
                // Not supposed to happen unless there is a bug.
                _ => panic!("Bug! Unexpectedly failed to compress!"),
            }
        }

        (output, ReturnCode::Ok)
    }
}

#[derive(Debug)]
enum Mode {
    Inflate,
    Deflate,
}

fn main() {
    let mut it = std::env::args();

    let _ = it.next().unwrap();

    let mode = match it.next().unwrap().as_str() {
        "inflate" => Mode::Inflate,
        "deflate" => Mode::Deflate,
        other => panic!("invalid mode {other:?}"),
    };

    let level: i32 = match mode {
        Mode::Inflate => 0,
        Mode::Deflate => it.next().unwrap().parse().unwrap(),
    };

    let implementation = it.next().unwrap().to_string();
    let path = it.next().unwrap();

    match implementation.as_str() {
        "og" => helper::<ZlibOg>(mode, &path, level),
        "ng" => helper::<ZlibNg>(mode, &path, level),
        "rs" => helper::<ZlibRs>(mode, &path, level),
        "cloudflare" => helper::<ZlibCloudflare>(mode, &path, level),
        "miniz" => helper::<MinizOxide>(mode, &path, level),
        other => panic!("invalid implementation: {other:?}"),
    };
}

fn helper<T: DeflateImplementation>(mode: Mode, path: &str, level: i32) {
    let mut output = vec![MaybeUninit::new(0u8); 1 << 28];
    let Ok(input) = std::fs::read(path) else {
        panic!("error opening {path:?}")
    };

    println!(
        "performing {mode:?} at level {level} using method {}",
        T::NAME
    );

    match mode {
        Mode::Inflate => {
            let config = InflateConfig { window_bits: 15 };
            T::uncompress_slice(&mut output, &input, config);
        }
        Mode::Deflate => {
            let config = DeflateConfig {
                level,
                method: Method::Deflated,
                window_bits: 15,
                mem_level: 8,
                strategy: Strategy::Default,
            };
            T::compress_slice(&mut output, &input, config);
        }
    }
}
