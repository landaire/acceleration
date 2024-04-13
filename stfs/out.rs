#![feature(prelude_import)]
#[prelude_import]
use std::prelude::rust_2021::*;
#[macro_use]
extern crate std;
mod consts {
    pub const HASHES_PER_HASH_TABLE: usize = 0xAA;
    pub const HASHES_PER_HASH_TABLE_LEVEL: [usize; 3] = [
        HASHES_PER_HASH_TABLE,
        HASHES_PER_HASH_TABLE * HASHES_PER_HASH_TABLE,
        HASHES_PER_HASH_TABLE * HASHES_PER_HASH_TABLE * HASHES_PER_HASH_TABLE,
    ];
    pub const DATA_BLOCKS_PER_HASH_TREE_LEVEL: [usize; 3] = [
        1,
        HASHES_PER_HASH_TABLE,
        HASHES_PER_HASH_TABLE * HASHES_PER_HASH_TABLE,
    ];
    pub const BLOCK_SIZE: usize = 0x1000;
    pub const MAX_IMAGE_SIZE: usize = 0x4000;
}
mod error {
    use thiserror::Error;
    pub enum StfsError {
        #[error("Invalid STFS package magic")]
        InvalidMagic,
        #[error("Invalid STFS package header")]
        InvalidHeader,
        #[error("Invalid package type")]
        InvalidPackageType,
        #[error("I/O error")]
        Io(#[from] std::io::Error),
        #[error("I/O error (binrw)")]
        Binrw(#[from] binrw::Error),
    }
    #[allow(unused_qualifications)]
    impl std::error::Error for StfsError {
        fn source(&self) -> ::core::option::Option<&(dyn std::error::Error + 'static)> {
            use thiserror::__private::AsDynError as _;
            #[allow(deprecated)]
            match self {
                StfsError::InvalidMagic { .. } => ::core::option::Option::None,
                StfsError::InvalidHeader { .. } => ::core::option::Option::None,
                StfsError::InvalidPackageType { .. } => ::core::option::Option::None,
                StfsError::Io { 0: source, .. } => {
                    ::core::option::Option::Some(source.as_dyn_error())
                }
                StfsError::Binrw { 0: source, .. } => {
                    ::core::option::Option::Some(source.as_dyn_error())
                }
            }
        }
    }
    #[allow(unused_qualifications)]
    impl ::core::fmt::Display for StfsError {
        fn fmt(&self, __formatter: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            #[allow(unused_variables, deprecated, clippy::used_underscore_binding)]
            match self {
                StfsError::InvalidMagic {} => {
                    __formatter.write_str("Invalid STFS package magic")
                }
                StfsError::InvalidHeader {} => {
                    __formatter.write_str("Invalid STFS package header")
                }
                StfsError::InvalidPackageType {} => {
                    __formatter.write_str("Invalid package type")
                }
                StfsError::Io(_0) => __formatter.write_str("I/O error"),
                StfsError::Binrw(_0) => __formatter.write_str("I/O error (binrw)"),
            }
        }
    }
    #[allow(unused_qualifications)]
    impl ::core::convert::From<std::io::Error> for StfsError {
        #[allow(deprecated)]
        fn from(source: std::io::Error) -> Self {
            StfsError::Io { 0: source }
        }
    }
    #[allow(unused_qualifications)]
    impl ::core::convert::From<binrw::Error> for StfsError {
        #[allow(deprecated)]
        fn from(source: binrw::Error) -> Self {
            StfsError::Binrw { 0: source }
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for StfsError {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            match self {
                StfsError::InvalidMagic => {
                    ::core::fmt::Formatter::write_str(f, "InvalidMagic")
                }
                StfsError::InvalidHeader => {
                    ::core::fmt::Formatter::write_str(f, "InvalidHeader")
                }
                StfsError::InvalidPackageType => {
                    ::core::fmt::Formatter::write_str(f, "InvalidPackageType")
                }
                StfsError::Io(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(f, "Io", &__self_0)
                }
                StfsError::Binrw(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "Binrw",
                        &__self_0,
                    )
                }
            }
        }
    }
}
pub mod fs {
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::SystemTime;
    use vfs::error::VfsErrorKind;
    use vfs::FileSystem;
    use vfs::VfsError;
    use crate::StfsEntry;
    use crate::StfsEntryRef;
    use crate::StfsFileEntry;
    use crate::StfsPackage;
    impl StfsPackage {
        fn find_file(&self, path: &str) -> vfs::VfsResult<StfsEntryRef> {
            let path = PathBuf::from(path);
            let mut current = Arc::clone(&self.files);
            for part in path.iter() {
                if part == "/" {
                    continue;
                }
                let current_copy = Arc::clone(&current);
                let node = current_copy.lock();
                match &*node {
                    crate::StfsEntry::File(_) => {
                        return Err(VfsErrorKind::FileNotFound.into());
                    }
                    crate::StfsEntry::Folder { entry, files } => {
                        {
                            ::std::io::_print(
                                format_args!("entry_name={0:#?}\n", node.name()),
                            );
                        };
                        if let Some(node) = files
                            .iter()
                            .find(|file| file.lock().name() == part.to_string_lossy())
                        {
                            current = Arc::clone(node)
                        } else {
                            return Err(VfsErrorKind::FileNotFound.into());
                        }
                    }
                }
            }
            Ok(current)
        }
    }
    impl FileSystem for StfsPackage {
        fn read_dir(
            &self,
            path: &str,
        ) -> vfs::VfsResult<Box<dyn Iterator<Item = String> + Send>> {
            let dir = self.find_file(path)?;
            let dir = dir.lock();
            if let StfsEntry::Folder { entry, files } = &*dir {
                Ok(
                    Box::new(
                        files
                            .iter()
                            .map(|file| file.lock().name())
                            .collect::<Vec<_>>()
                            .into_iter(),
                    ),
                )
            } else {
                {
                    ::core::panicking::panic_fmt(
                        format_args!(
                            "internal error: entered unreachable code: {0}",
                            format_args!("we should never have a file here"),
                        ),
                    );
                }
            }
        }
        fn create_dir(&self, path: &str) -> vfs::VfsResult<()> {
            ::core::panicking::panic("not yet implemented")
        }
        fn open_file(
            &self,
            path: &str,
        ) -> vfs::VfsResult<Box<dyn vfs::SeekAndRead + Send>> {
            ::core::panicking::panic("not yet implemented")
        }
        fn create_file(
            &self,
            path: &str,
        ) -> vfs::VfsResult<Box<dyn vfs::SeekAndWrite + Send>> {
            ::core::panicking::panic("not yet implemented")
        }
        fn append_file(
            &self,
            path: &str,
        ) -> vfs::VfsResult<Box<dyn vfs::SeekAndWrite + Send>> {
            ::core::panicking::panic("not yet implemented")
        }
        fn metadata(&self, path: &str) -> vfs::VfsResult<vfs::VfsMetadata> {
            let file = self.find_file(path)?;
            let file = &*file.lock();
            let metadata = match file {
                StfsEntry::File(entry) => {
                    let attr = entry.file_attributes.as_ref().unwrap();
                    {
                        ::std::io::_print(
                            format_args!(
                                "{0:#?}\n",
                                crate::util::stf_timestamp_to_chrono(
                                    attr.created_time_stamp,
                                ),
                            ),
                        );
                    };
                    vfs::VfsMetadata {
                        file_type: vfs::VfsFileType::File,
                        len: attr.file_size as u64,
                        created: Some(
                            crate::util::stf_timestamp_to_chrono(attr.created_time_stamp)
                                .into(),
                        ),
                        modified: None,
                        accessed: Some(
                            crate::util::stf_timestamp_to_chrono(attr.access_time_stamp)
                                .into(),
                        ),
                    }
                }
                StfsEntry::Folder { entry, files } => {
                    let attr = entry.file_attributes.as_ref().unwrap();
                    {
                        ::std::io::_print(
                            format_args!(
                                "{0:#?}\n",
                                crate::util::stf_timestamp_to_chrono(
                                    attr.created_time_stamp,
                                ),
                            ),
                        );
                    };
                    vfs::VfsMetadata {
                        file_type: vfs::VfsFileType::Directory,
                        len: 0,
                        created: Some(
                            crate::util::stf_timestamp_to_chrono(attr.created_time_stamp)
                                .into(),
                        ),
                        modified: None,
                        accessed: Some(
                            crate::util::stf_timestamp_to_chrono(attr.access_time_stamp)
                                .into(),
                        ),
                    }
                }
            };
            Ok(metadata)
        }
        fn exists(&self, path: &str) -> vfs::VfsResult<bool> {
            ::core::panicking::panic("not yet implemented")
        }
        fn remove_file(&self, path: &str) -> vfs::VfsResult<()> {
            ::core::panicking::panic("not yet implemented")
        }
        fn remove_dir(&self, path: &str) -> vfs::VfsResult<()> {
            ::core::panicking::panic("not yet implemented")
        }
    }
}
mod parse {
    use binrw::binrw;
    use binrw::BinReaderExt;
    use binrw::NullString;
    use binrw::NullWideString;
    use modular_bitfield::prelude::*;
    use std::collections::HashMap;
    use std::io::Read;
    use std::io::Write;
    use std::ops::Range;
    use std::ops::self;
    use std::sync::Arc;
    use crate::consts::*;
    use bitflags::bitflags;
    use chrono::DateTime;
    use chrono::Utc;
    use parking_lot::Mutex;
    use serde::Deserialize;
    use serde::Serialize;
    use serde::Serializer;
    use std::io::Cursor;
    use thiserror::Error;
    use variantly::Variantly;
    use crate::error::StfsError;
    use crate::sparse_reader::SparseReader;
    use crate::util::*;
    pub type StfsEntryRef = Arc<Mutex<StfsEntry>>;
    const BLOCK_SIZE: usize = 0x1000;
    pub enum PackageType {
        /// User container packages that are created by an Xbox 360 console and
        /// signed by the user's private key.
        Con,
        /// Xbox LIVE-distributed package that is signed by Microsoft's private key.
        Live,
        /// Offline-distributed package that is signed by Microsoft's private key.
        Pirs,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for PackageType {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                match (|| {
                    match &binrw::BinRead::read_options(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        (),
                    )? {
                        b"CON " => Ok(Self::Con),
                        b"LIVE" => Ok(Self::Live),
                        b"PIRS" => Ok(Self::Pirs),
                        _ => {
                            Err(binrw::Error::NoVariantMatch {
                                pos: __binrw_generated_position_temp,
                            })
                        }
                    }
                })() {
                    v @ Ok(_) => return v,
                    Err(__binrw_temp) => {
                        binrw::__private::restore_position_variant(
                            __binrw_generated_var_reader,
                            __binrw_generated_position_temp,
                            __binrw_temp,
                        )?;
                    }
                }
                Err(binrw::Error::NoVariantMatch {
                    pos: __binrw_generated_position_temp,
                })
            })()
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for PackageType {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            match self {
                Self::Con => {
                    binrw::BinWrite::write_options(
                        &b"CON ",
                        __binrw_generated_var_writer,
                        __binrw_generated_var_endian,
                        (),
                    )?;
                }
                Self::Live => {
                    binrw::BinWrite::write_options(
                        &b"LIVE",
                        __binrw_generated_var_writer,
                        __binrw_generated_var_endian,
                        (),
                    )?;
                }
                Self::Pirs => {
                    binrw::BinWrite::write_options(
                        &b"PIRS",
                        __binrw_generated_var_writer,
                        __binrw_generated_var_endian,
                        (),
                    )?;
                }
            }
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for PackageType {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    PackageType::Con => "Con",
                    PackageType::Live => "Live",
                    PackageType::Pirs => "Pirs",
                },
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for PackageType {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    PackageType::Con => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "PackageType",
                            0u32,
                            "Con",
                        )
                    }
                    PackageType::Live => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "PackageType",
                            1u32,
                            "Live",
                        )
                    }
                    PackageType::Pirs => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "PackageType",
                            2u32,
                            "Pirs",
                        )
                    }
                }
            }
        }
    };
    #[automatically_derived]
    impl ::core::marker::StructuralPartialEq for PackageType {}
    #[automatically_derived]
    impl ::core::cmp::PartialEq for PackageType {
        #[inline]
        fn eq(&self, other: &PackageType) -> bool {
            let __self_tag = ::core::intrinsics::discriminant_value(self);
            let __arg1_tag = ::core::intrinsics::discriminant_value(other);
            __self_tag == __arg1_tag
        }
    }
    #[automatically_derived]
    impl ::core::cmp::Eq for PackageType {
        #[inline]
        #[doc(hidden)]
        #[coverage(off)]
        fn assert_receiver_is_total_eq(&self) -> () {}
    }
    #[automatically_derived]
    impl ::core::marker::Copy for PackageType {}
    #[automatically_derived]
    impl ::core::clone::Clone for PackageType {
        #[inline]
        fn clone(&self) -> PackageType {
            *self
        }
    }
    pub enum StfsEntry {
        File(StfsFileEntry),
        Folder { entry: StfsFileEntry, files: Vec<StfsEntryRef> },
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for StfsEntry {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            match self {
                StfsEntry::File(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "File",
                        &__self_0,
                    )
                }
                StfsEntry::Folder { entry: __self_0, files: __self_1 } => {
                    ::core::fmt::Formatter::debug_struct_field2_finish(
                        f,
                        "Folder",
                        "entry",
                        __self_0,
                        "files",
                        &__self_1,
                    )
                }
            }
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for StfsEntry {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    StfsEntry::File(ref __field0) => {
                        _serde::Serializer::serialize_newtype_variant(
                            __serializer,
                            "StfsEntry",
                            0u32,
                            "File",
                            __field0,
                        )
                    }
                    StfsEntry::Folder { ref entry, ref files } => {
                        let mut __serde_state = _serde::Serializer::serialize_struct_variant(
                            __serializer,
                            "StfsEntry",
                            1u32,
                            "Folder",
                            0 + 1 + 1,
                        )?;
                        _serde::ser::SerializeStructVariant::serialize_field(
                            &mut __serde_state,
                            "entry",
                            entry,
                        )?;
                        _serde::ser::SerializeStructVariant::serialize_field(
                            &mut __serde_state,
                            "files",
                            files,
                        )?;
                        _serde::ser::SerializeStructVariant::end(__serde_state)
                    }
                }
            }
        }
    };
    impl StfsEntry {
        pub fn file(self) -> std::option::Option<((StfsFileEntry))> {
            match self {
                StfsEntry::File(ident_695e54f41e0e4afab282d846b292b0aa) => {
                    std::option::Option::Some(((ident_695e54f41e0e4afab282d846b292b0aa)))
                }
                _ => std::option::Option::None,
            }
        }
        pub fn file_ref(&self) -> std::option::Option<((&StfsFileEntry))> {
            match self {
                StfsEntry::File(ident_695e54f41e0e4afab282d846b292b0aa) => {
                    std::option::Option::Some(((ident_695e54f41e0e4afab282d846b292b0aa)))
                }
                _ => std::option::Option::None,
            }
        }
        pub fn file_mut(&mut self) -> std::option::Option<((&mut StfsFileEntry))> {
            match self {
                StfsEntry::File(ident_695e54f41e0e4afab282d846b292b0aa) => {
                    std::option::Option::Some(((ident_695e54f41e0e4afab282d846b292b0aa)))
                }
                _ => std::option::Option::None,
            }
        }
        pub fn file_or<E>(self, or: E) -> std::result::Result<((StfsFileEntry)), E> {
            self.file_or_else(|| or)
        }
        pub fn file_or_else<E, F: std::ops::FnOnce() -> E>(
            self,
            or_else: F,
        ) -> std::result::Result<((StfsFileEntry)), E> {
            match self {
                StfsEntry::File(ident_695e54f41e0e4afab282d846b292b0aa) => {
                    std::result::Result::Ok(((ident_695e54f41e0e4afab282d846b292b0aa)))
                }
                _ => std::result::Result::Err(or_else()),
            }
        }
        pub fn file_ref_or<E>(
            &self,
            or: E,
        ) -> std::result::Result<((&StfsFileEntry)), E> {
            self.file_ref_or_else(|| or)
        }
        pub fn file_mut_or<E>(
            &mut self,
            or: E,
        ) -> std::result::Result<((&mut StfsFileEntry)), E> {
            self.file_mut_or_else(|| or)
        }
        pub fn file_ref_or_else<E, F: std::ops::FnOnce() -> E>(
            &self,
            or_else: F,
        ) -> std::result::Result<((&StfsFileEntry)), E> {
            match self {
                StfsEntry::File(ident_695e54f41e0e4afab282d846b292b0aa) => {
                    std::result::Result::Ok(((ident_695e54f41e0e4afab282d846b292b0aa)))
                }
                _ => std::result::Result::Err(or_else()),
            }
        }
        pub fn file_mut_or_else<E, F: std::ops::FnOnce() -> E>(
            &mut self,
            or_else: F,
        ) -> std::result::Result<((&mut StfsFileEntry)), E> {
            match self {
                StfsEntry::File(ident_695e54f41e0e4afab282d846b292b0aa) => {
                    std::result::Result::Ok(((ident_695e54f41e0e4afab282d846b292b0aa)))
                }
                _ => std::result::Result::Err(or_else()),
            }
        }
        pub fn and_then_file<
            F: std::ops::FnOnce(((StfsFileEntry))) -> ((StfsFileEntry)),
        >(self, and_then: F) -> Self {
            match self {
                StfsEntry::File(ident_695e54f41e0e4afab282d846b292b0aa) => {
                    let (ident_695e54f41e0e4afab282d846b292b0aa) = and_then(
                        (ident_695e54f41e0e4afab282d846b292b0aa),
                    );
                    StfsEntry::File(ident_695e54f41e0e4afab282d846b292b0aa)
                }
                _ => self,
            }
        }
        pub fn expect_file(self, msg: &str) -> ((StfsFileEntry)) {
            self.unwrap_or_else_file(|| {
                ::std::rt::panic_display(&msg);
            })
        }
        #[deprecated(
            since = "0.2.0",
            note = "Please use the derived `StfsEntry::file` method instead. This method will be removed in 1.0.0 or next pre-stable minor bump."
        )]
        pub fn ok_file(self) -> std::option::Option<((StfsFileEntry))> {
            self.file()
        }
        #[deprecated(
            since = "0.2.0",
            note = "Please use the derived `StfsEntry::file_or` method instead. This method will be removed in 1.0.0 or next pre-stable minor bump."
        )]
        pub fn ok_or_file<E>(self, or: E) -> std::result::Result<((StfsFileEntry)), E> {
            self.file_or(or)
        }
        #[deprecated(
            since = "0.2.0",
            note = "Please use the derived `StfsEntry::file_or_else` method instead. This method will be removed in 1.0.0 or next pre-stable minor bump."
        )]
        pub fn ok_or_else_file<E, F: std::ops::FnOnce() -> E>(
            self,
            or_else: F,
        ) -> std::result::Result<((StfsFileEntry)), E> {
            self.file_or_else(or_else)
        }
        pub fn or_else_file<F: std::ops::FnOnce() -> ((StfsFileEntry))>(
            self,
            or_else: F,
        ) -> Self {
            match self {
                StfsEntry::File(ident_695e54f41e0e4afab282d846b292b0aa) => {
                    StfsEntry::File(ident_695e54f41e0e4afab282d846b292b0aa)
                }
                _ => {
                    let (ident_695e54f41e0e4afab282d846b292b0aa) = or_else();
                    StfsEntry::File(ident_695e54f41e0e4afab282d846b292b0aa)
                }
            }
        }
        pub fn unwrap_file(self) -> ((StfsFileEntry)) {
            self.unwrap_or_else_file(|| { ::std::rt::begin_panic("explicit panic") })
        }
        pub fn unwrap_or_file(self, or: ((StfsFileEntry))) -> ((StfsFileEntry)) {
            self.unwrap_or_else_file(|| or)
        }
        pub fn unwrap_or_else_file<F: std::ops::FnOnce() -> ((StfsFileEntry))>(
            self,
            or_else: F,
        ) -> ((StfsFileEntry)) {
            match self {
                StfsEntry::File(ident_695e54f41e0e4afab282d846b292b0aa) => {
                    ((ident_695e54f41e0e4afab282d846b292b0aa))
                }
                _ => or_else(),
            }
        }
        pub fn is_file(&self) -> bool {
            match self {
                StfsEntry::File(..) => true,
                _ => false,
            }
        }
        pub fn is_not_file(&self) -> bool {
            !self.is_file()
        }
        pub fn and_file(self, and: Self) -> Self {
            match (&self, &and) {
                (StfsEntry::File(..), StfsEntry::File(..)) => and,
                _ => self,
            }
        }
        pub fn or_file(self, or: Self) -> Self {
            match &self {
                StfsEntry::File(..) => self,
                _ => or,
            }
        }
        pub fn is_folder(&self) -> bool {
            match self {
                StfsEntry::Folder { .. } => true,
                _ => false,
            }
        }
        pub fn is_not_folder(&self) -> bool {
            !self.is_folder()
        }
        pub fn and_folder(self, and: Self) -> Self {
            match (&self, &and) {
                (StfsEntry::Folder { .. }, StfsEntry::Folder { .. }) => and,
                _ => self,
            }
        }
        pub fn or_folder(self, or: Self) -> Self {
            match &self {
                StfsEntry::Folder { .. } => self,
                _ => or,
            }
        }
    }
    impl StfsEntry {
        pub fn name(&self) -> String {
            self.entry().name.to_string()
        }
        pub fn entry(&self) -> &StfsFileEntry {
            match self {
                StfsEntry::File(entry) | StfsEntry::Folder { entry, files: _ } => entry,
            }
        }
    }
    pub enum StfsPackageSex {
        Female = 0,
        Male,
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for StfsPackageSex {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    StfsPackageSex::Female => "Female",
                    StfsPackageSex::Male => "Male",
                },
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for StfsPackageSex {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    StfsPackageSex::Female => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "StfsPackageSex",
                            0u32,
                            "Female",
                        )
                    }
                    StfsPackageSex::Male => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "StfsPackageSex",
                            1u32,
                            "Male",
                        )
                    }
                }
            }
        }
    };
    #[automatically_derived]
    impl ::core::marker::Copy for StfsPackageSex {}
    #[automatically_derived]
    impl ::core::clone::Clone for StfsPackageSex {
        #[inline]
        fn clone(&self) -> StfsPackageSex {
            *self
        }
    }
    impl StfsPackageSex {
        /// The "block step" depends on the package's "sex". This basically determines
        /// which hash tables are used.
        const fn block_step(&self) -> [usize; 2] {
            match self {
                StfsPackageSex::Female => [0xAB, 0x718F],
                StfsPackageSex::Male => [0xAC, 0x723A],
            }
        }
    }
    struct HashEntry {
        block_hash: [u8; 0x14],
        status: u8,
        next_block: Block,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for HashEntry {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut block_hash: [u8; 0x14] = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'block_hash' in HashEntry"
                                .into(),
                            line: 88u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄───╮\n   \u{1b}[1m88\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mblock_hash: [\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m; \u{1b}[0m\u{1b}[1m\u{1b}[38;5;135m0x14\u{1b}[39m\u{1b}[0m\u{1b}[1m]\u{1b}[0m\n  ┄───╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut status: u8 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'status' in HashEntry".into(),
                            line: 89u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄───╮\n   \u{1b}[1m89\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mstatus: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\n  ┄───╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut next_block: Block = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'next_block' in HashEntry"
                                .into(),
                            line: 90u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄───╮\n   \u{1b}[1m90\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mnext_block: Block\u{1b}[0m\n  ┄───╯\n",
                            ),
                        },
                    ))?;
                let __binrw_this = Self {
                    block_hash,
                    status,
                    next_block,
                };
                Ok(__binrw_this)
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for HashEntry {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_this = self;
            let HashEntry { ref block_hash, ref status, ref next_block } = self;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_block_hash: <[u8; 0x14] as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &block_hash,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_block_hash,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_status: <u8 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &status,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_status,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_next_block: <Block as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &next_block,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_next_block,
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::default::Default for HashEntry {
        #[inline]
        fn default() -> HashEntry {
            HashEntry {
                block_hash: ::core::default::Default::default(),
                status: ::core::default::Default::default(),
                next_block: ::core::default::Default::default(),
            }
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for HashEntry {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field3_finish(
                f,
                "HashEntry",
                "block_hash",
                &self.block_hash,
                "status",
                &self.status,
                "next_block",
                &&self.next_block,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for HashEntry {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "HashEntry",
                    false as usize + 1 + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "block_hash",
                    &self.block_hash,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "status",
                    &self.status,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "next_block",
                    &self.next_block,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    pub struct HashTableMeta {
        pub block_step: [usize; 2],
        pub tables_per_level: [usize; 3],
        pub top_table: HashTable,
        pub first_table_address: usize,
    }
    #[automatically_derived]
    impl ::core::default::Default for HashTableMeta {
        #[inline]
        fn default() -> HashTableMeta {
            HashTableMeta {
                block_step: ::core::default::Default::default(),
                tables_per_level: ::core::default::Default::default(),
                top_table: ::core::default::Default::default(),
                first_table_address: ::core::default::Default::default(),
            }
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for HashTableMeta {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field4_finish(
                f,
                "HashTableMeta",
                "block_step",
                &self.block_step,
                "tables_per_level",
                &self.tables_per_level,
                "top_table",
                &self.top_table,
                "first_table_address",
                &&self.first_table_address,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for HashTableMeta {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "HashTableMeta",
                    false as usize + 1 + 1 + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "block_step",
                    &self.block_step,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "tables_per_level",
                    &self.tables_per_level,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "top_table",
                    &self.top_table,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "first_table_address",
                    &self.first_table_address,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    impl HashTableMeta {
        pub fn new(
            sex: StfsPackageSex,
            header: &XContentHeader,
        ) -> Result<Self, StfsError> {
            let mut meta = HashTableMeta::default();
            meta.block_step = sex.block_step();
            meta
                .first_table_address = ((header.header_size as usize) + 0x0FFF)
                & 0xFFFF_F000;
            let stfs_vol = header
                .volume_descriptor
                .stfs_ref()
                .expect("volume descriptor does not represent an STFS filesystem");
            let allocated_block_count = stfs_vol.allocated_block_count as usize;
            meta
                .tables_per_level[0] = ((allocated_block_count as usize)
                / HASHES_PER_HASH_TABLE)
                + if (allocated_block_count as usize) % HASHES_PER_HASH_TABLE != 0 {
                    1
                } else {
                    0
                };
            meta
                .tables_per_level[1] = (meta.tables_per_level[1] / HASHES_PER_HASH_TABLE)
                + if meta.tables_per_level[1] % HASHES_PER_HASH_TABLE != 0
                    && allocated_block_count > HASHES_PER_HASH_TABLE
                {
                    1
                } else {
                    0
                };
            meta
                .tables_per_level[2] = (meta.tables_per_level[2] / HASHES_PER_HASH_TABLE)
                + if meta.tables_per_level[2] % HASHES_PER_HASH_TABLE != 0
                    && allocated_block_count > DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]
                {
                    1
                } else {
                    0
                };
            meta.top_table.level = header.root_hash_table_level()?;
            meta
                .top_table
                .true_block_number = meta
                .compute_backing_hash_block_number_for_level(
                    Block(0),
                    meta.top_table.level,
                    sex,
                );
            let base_address = (meta.top_table.true_block_number.0 * BLOCK_SIZE)
                + meta.first_table_address;
            meta
                .top_table
                .address_in_file = base_address
                + ((stfs_vol.flags.root_active_index() as usize) << 0xC);
            meta
                .top_table
                .entry_count = (allocated_block_count as usize)
                / DATA_BLOCKS_PER_HASH_TREE_LEVEL[meta.top_table.level as usize];
            if (allocated_block_count > DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]
                && allocated_block_count % DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] != 0)
                || (allocated_block_count > HASHES_PER_HASH_TABLE
                    && allocated_block_count % HASHES_PER_HASH_TABLE != 0)
            {
                meta.top_table.entry_count += 1;
            }
            meta.top_table.entries.reserve(meta.top_table.entry_count);
            Ok(meta)
        }
        pub fn compute_backing_hash_block_number_for_level(
            &self,
            block: Block,
            level: HashTableLevel,
            sex: StfsPackageSex,
        ) -> Block {
            match level {
                HashTableLevel::First => {
                    self.compute_first_level_backing_hash_block_number(block, sex)
                }
                HashTableLevel::Second => {
                    self.compute_second_level_backing_hash_block_number(block, sex)
                }
                HashTableLevel::Third => {
                    self.compute_third_level_backing_hash_block_number()
                }
            }
        }
        pub fn compute_first_level_backing_hash_block_number(
            &self,
            block: Block,
            sex: StfsPackageSex,
        ) -> Block {
            if block.0 < HASHES_PER_HASH_TABLE {
                return Block(0);
            }
            let mut block_number = (block.0 / HASHES_PER_HASH_TABLE)
                * self.block_step[0];
            block_number
                += ((block.0 / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]) + 1) << (sex as u8);
            let block = if block.0 / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] == 0 {
                block_number
            } else {
                block_number + (1 << (sex as u8))
            };
            block.into()
        }
        pub fn compute_second_level_backing_hash_block_number(
            &self,
            block: Block,
            sex: StfsPackageSex,
        ) -> Block {
            let block = if block.0 < DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] {
                self.block_step[0]
            } else {
                (1 << (sex as u8))
                    + (block.0 / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]) * self.block_step[1]
            };
            block.into()
        }
        pub fn compute_third_level_backing_hash_block_number(&self) -> Block {
            self.block_step[1].into()
        }
    }
    pub struct StfsPackage {
        pub header: XContentHeader,
        pub hash_table_meta: HashTableMeta,
        pub files: StfsEntryRef,
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for StfsPackage {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field3_finish(
                f,
                "StfsPackage",
                "header",
                &self.header,
                "hash_table_meta",
                &self.hash_table_meta,
                "files",
                &&self.files,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for StfsPackage {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "StfsPackage",
                    false as usize + 1 + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "header",
                    &self.header,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "hash_table_meta",
                    &self.hash_table_meta,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "files",
                    &self.files,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    impl TryFrom<&[u8]> for StfsPackage {
        type Error = StfsError;
        fn try_from(input: &[u8]) -> Result<Self, Self::Error> {
            let mut cursor = Cursor::new(input);
            let xcontent_header = cursor.read_be::<XContentHeader>()?;
            let mut hash_table_meta = HashTableMeta::new(
                xcontent_header.sex(),
                &xcontent_header,
            )?;
            hash_table_meta
                .top_table
                .parse_hash_entries(&input[hash_table_meta.top_table.data_range()])?;
            let mut package = StfsPackage {
                header: xcontent_header,
                hash_table_meta,
                files: Arc::new(
                    Mutex::new(StfsEntry::Folder {
                        entry: Default::default(),
                        files: Default::default(),
                    }),
                ),
            };
            package.read_files(input)?;
            Ok(package)
        }
    }
    pub struct Block(usize);
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for Block {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_generated_read_function = binrw::__private::parse_fn_type_hint(
                    binrw::helpers::read_u24,
                );
                let mut __binrw_generated_map_func_self_0 = (binrw::__private::coerce_fn::<
                    usize,
                    _,
                    _,
                >(|block: u32| block as usize));
                let mut self_0: usize = __binrw_generated_map_func_self_0(
                    (|| {
                        __binrw_generated_read_function
                    })()(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'self_0' in Block".into(),
                                line: 230u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   232 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mparse_with\u{1b}[39m = \u{1b}[38;5;197mbinrw\u{1b}[39m::helpers::read_u24, \u{1b}[38;5;197mmap\u{1b}[39m = |block: \u{1b}[38;5;197mu32\u{1b}[39m| block as \u{1b}[38;5;197musize\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   233 |  \u{1b}[38;5;243m// TODO: write u24\u{1b}[39m\n   234 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39mmap = |block: &usize| *block as u32 \u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m //, write_with = binrw::helpers::write_u24)]\n   \u{1b}[1m235\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1m\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197musize\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?,
                );
                let __binrw_this = Self(self_0);
                Ok(__binrw_this)
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for Block {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_this = self;
            let Block(ref self_0) = self;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            let __binrw_generated_map_func_self_0 = binrw::__private::write_map_fn_input_type_hint::<
                &usize,
                _,
                _,
            >((|block: &usize| *block as u32));
            let __binrw_generated_write_function = binrw::__private::write_fn_map_output_type_hint(
                &__binrw_generated_map_func_self_0,
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_self_0 = binrw::__private::write_map_args_type_hint(
                &__binrw_generated_map_func_self_0,
                <_ as binrw::__private::Required>::args(),
            );
            let self_0 = __binrw_generated_map_func_self_0(self_0);
            __binrw_generated_write_function(
                &self_0,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_self_0,
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::default::Default for Block {
        #[inline]
        fn default() -> Block {
            Block(::core::default::Default::default())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for Block {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_tuple_field1_finish(f, "Block", &&self.0)
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for Block {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                _serde::Serializer::serialize_newtype_struct(
                    __serializer,
                    "Block",
                    &self.0,
                )
            }
        }
    };
    #[automatically_derived]
    impl ::core::marker::Copy for Block {}
    #[automatically_derived]
    impl ::core::clone::Clone for Block {
        #[inline]
        fn clone(&self) -> Block {
            let _: ::core::clone::AssertParamIsClone<usize>;
            *self
        }
    }
    impl From<usize> for Block {
        fn from(value: usize) -> Self {
            Block(value)
        }
    }
    impl ops::Add<Block> for Block {
        type Output = Block;
        fn add(self, rhs: Block) -> Self::Output {
            Block(self.0 + rhs.0)
        }
    }
    impl ops::Add<usize> for Block {
        type Output = Block;
        fn add(self, rhs: usize) -> Self::Output {
            Block(self.0 + rhs)
        }
    }
    impl ops::Mul<usize> for Block {
        type Output = Block;
        fn mul(self, rhs: usize) -> Self::Output {
            Block(self.0 * rhs)
        }
    }
    impl StfsPackage {
        pub fn file_ranges(
            &self,
            entry: &StfsFileEntry,
            input: &[u8],
        ) -> Result<Vec<Range<u64>>, StfsError> {
            let mut mappings = Vec::new();
            if entry.file_attributes.is_none() {
                return Ok(Vec::new());
            }
            let attributes = entry.file_attributes.as_ref().unwrap();
            if attributes.file_size == 0 {
                return Ok(Vec::new());
            }
            let start_address = self.block_to_addr(attributes.starting_block);
            let mut next_address = start_address;
            let mut data_remaining = attributes.file_size as u64;
            if entry.flags.has_consecutive_blocks() {
                let blocks_until_hash_table = (self
                    .hash_table_meta
                    .compute_first_level_backing_hash_block_number(
                        attributes.starting_block,
                        self.header.sex(),
                    )
                    .0 + self.hash_table_meta.block_step[0])
                    - (((start_address as usize)
                        - self.hash_table_meta.first_table_address) / BLOCK_SIZE);
                if attributes.block_count as usize <= blocks_until_hash_table {
                    mappings
                        .push(
                            start_address..(start_address + attributes.file_size as u64),
                        );
                } else {
                    while data_remaining > 0 {
                        let read_len = std::cmp::min(
                            HASHES_PER_HASH_TABLE * BLOCK_SIZE,
                            data_remaining as usize,
                        ) as u64;
                        let range = next_address..(next_address + read_len);
                        mappings.push(range.clone());
                        let data_read = range.end - range.start;
                        data_remaining -= data_read;
                        next_address += data_read;
                        next_address
                            += self.hash_table_skip_for_address(next_address as usize)?
                                as u64;
                    }
                }
            } else {
                let mut data_remaining = attributes.file_size as u64;
                let mut block_count = data_remaining / (BLOCK_SIZE as u64);
                if data_remaining % (BLOCK_SIZE as u64) != 0 {
                    block_count += 1;
                }
                let mut block = attributes.starting_block;
                for _ in 0..block_count {
                    let read_len = std::cmp::min(BLOCK_SIZE as u64, data_remaining);
                    let block_address = self.block_to_addr(block);
                    mappings.push(block_address..(block_address + read_len));
                    let hash_entry = self.block_hash_entry(block, input)?;
                    block = hash_entry.next_block;
                    data_remaining -= read_len;
                }
            }
            Ok(mappings)
        }
        fn hash_table_skip_for_address(
            &self,
            table_address: usize,
        ) -> Result<usize, StfsError> {
            let sex = self.header.sex() as usize;
            let mut block_number = (table_address
                - self.hash_table_meta.first_table_address) / BLOCK_SIZE;
            if block_number == 0 {
                return Ok(BLOCK_SIZE << sex);
            }
            if block_number == self.hash_table_meta.block_step[1] {
                return Ok((BLOCK_SIZE * 3) << sex);
            } else if block_number > self.hash_table_meta.block_step[1] {
                block_number -= self.hash_table_meta.block_step[1] + (1 << sex);
            }
            if block_number == self.hash_table_meta.block_step[0]
                || block_number % self.hash_table_meta.block_step[1] == 0
            {
                Ok((BLOCK_SIZE * 2) << sex)
            } else {
                Ok(BLOCK_SIZE << sex)
            }
        }
        fn block_hash_entry(
            &self,
            block: Block,
            input: &[u8],
        ) -> Result<HashEntry, StfsError> {
            if let Some(stfs_vol) = self.header.volume_descriptor.stfs_ref() {
                if block.0 > stfs_vol.allocated_block_count as usize {
                    {
                        ::core::panicking::panic_fmt(
                            format_args!(
                                "Reference to illegal block number: {0:#x} ({1:#x} allocated)",
                                block.0,
                                stfs_vol.allocated_block_count,
                            ),
                        );
                    };
                }
                let mut reader = Cursor::new(input);
                reader.set_position(self.block_hash_address(block, input)?);
                Ok(reader.read_be::<HashEntry>()?)
            } else {
                {
                    ::core::panicking::panic_fmt(format_args!("invalid volume type"));
                };
            }
        }
        fn block_hash_address(
            &self,
            block: Block,
            input: &[u8],
        ) -> Result<u64, StfsError> {
            if let Some(stfs_vol) = self.header.volume_descriptor.stfs_ref() {
                if block.0 > stfs_vol.allocated_block_count as usize {
                    {
                        ::core::panicking::panic_fmt(
                            format_args!(
                                "Reference to illegal block number: {0:#x} ({1:#x} allocated)",
                                block.0,
                                stfs_vol.allocated_block_count,
                            ),
                        );
                    };
                }
                let mut hash_addr = (self
                    .hash_table_meta
                    .compute_first_level_backing_hash_block_number(
                        block,
                        self.header.sex(),
                    )
                    .0 * BLOCK_SIZE) + self.hash_table_meta.first_table_address;
                hash_addr += (block.0 % HASHES_PER_HASH_TABLE) * 0x18;
                let address = match self.hash_table_meta.top_table.level {
                    HashTableLevel::First => {
                        hash_addr as u64
                            + ((stfs_vol.flags.root_active_index() as u64) << 0xC)
                    }
                    HashTableLevel::Second => {
                        hash_addr as u64
                            + ((self
                                .hash_table_meta
                                .top_table
                                .entries[block.0 / DATA_BLOCKS_PER_HASH_TREE_LEVEL[1]]
                                .status as u64 & 0x40) << 6)
                    }
                    HashTableLevel::Third => {
                        let first_level_offset = (self
                            .hash_table_meta
                            .top_table
                            .entries[block.0 / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]]
                            .status as u64 & 0x40) << 6;
                        let position = (self
                            .hash_table_meta
                            .compute_second_level_backing_hash_block_number(
                                block,
                                self.header.sex(),
                            )
                            .0 * BLOCK_SIZE) + self.hash_table_meta.first_table_address
                            + first_level_offset as usize
                            + ((block.0 % DATA_BLOCKS_PER_HASH_TREE_LEVEL[1]) * 0x18);
                        let status_byte = input[position + 0x14];
                        hash_addr as u64 + ((status_byte as u64 & 0x40) << 0x6)
                    }
                };
                Ok(address)
            } else {
                {
                    ::core::panicking::panic_fmt(format_args!("invalid filesystem"));
                }
            }
        }
        fn read_files(&mut self, input: &[u8]) -> Result<(), StfsError> {
            let stfs_vol = self
                .header
                .volume_descriptor
                .stfs_ref()
                .expect("volume descriptor is not an STFS file");
            let mut reader = Cursor::new(input);
            let mut block = stfs_vol.file_table_block_num;
            let mut folders = HashMap::<u16, StfsEntryRef>::new();
            let mut files = Vec::new();
            folders
                .insert(
                    0xffff,
                    Arc::new(
                        Mutex::new(StfsEntry::Folder {
                            entry: StfsFileEntry::default(),
                            files: Vec::new(),
                        }),
                    ),
                );
            for block_idx in 0..(stfs_vol.file_table_block_count as usize) {
                {
                    ::std::io::_print(format_args!("block: {0:#X?}\n", block));
                };
                let current_addr = self.block_to_addr(block);
                {
                    ::std::io::_print(format_args!("addr: {0:#X?}\n", current_addr));
                };
                reader.set_position(current_addr);
                for file_entry_idx in 0..0x40 {
                    let addressing_info = StfsFileEntryAddressingInfo {
                        file_entry_address: current_addr
                            + (file_entry_idx as u64 * 0x40),
                        file_table_index: (block_idx * 0x40) + file_entry_idx,
                    };
                    {
                        ::std::io::_print(format_args!("reading file entry\n"));
                    };
                    let mut entry = reader.read_be::<StfsFileEntry>()?;
                    if entry.flags.name_len() == 0 {
                        break;
                    }
                    let file_table_index = addressing_info.file_table_index;
                    entry.addressing_info = addressing_info;
                    if entry.flags.is_folder() {
                        let entry_idx = file_table_index;
                        let folder = Arc::new(
                            Mutex::new(StfsEntry::Folder {
                                entry,
                                files: Vec::new(),
                            }),
                        );
                        folders.insert(entry_idx as u16, folder.clone());
                        files.push(folder.clone());
                    } else {
                        files.push(Arc::new(Mutex::new(StfsEntry::File(entry))));
                    }
                }
                block = self.block_hash_entry(block.into(), input)?.next_block;
            }
            for file in files.drain(..) {
                if let StfsEntry::File(entry) | StfsEntry::Folder { entry, files: _ } = &*file
                    .lock()
                {
                    if let Some(attributes) = entry.file_attributes.as_ref() {
                        let cached_entry = folders.get(&attributes.dirent);
                        if let Some(entry) = cached_entry {
                            if let StfsEntry::Folder { entry: _, files } = &mut *entry
                                .lock()
                            {
                                files.push(file.clone());
                            }
                        } else {
                            {
                                ::core::panicking::panic_fmt(
                                    format_args!(
                                        "Corrupt STFS file: missing folder dirent {0:#x}",
                                        attributes.dirent,
                                    ),
                                );
                            };
                        }
                    }
                }
            }
            self.files = folders.remove(&0xffff).expect("no root file entry");
            Ok(())
        }
        fn block_to_addr(&self, block: Block) -> u64 {
            if block.0 > 2usize.pow(24) - 1 {
                {
                    ::core::panicking::panic_fmt(
                        format_args!("invalid block: {0:#x}", block.0),
                    );
                };
            }
            (self.compute_data_block_num(block) * BLOCK_SIZE as u64)
                + self.hash_table_meta.first_table_address as u64
        }
        fn compute_data_block_num(&self, block: Block) -> u64 {
            let sex = self.header.sex() as usize;
            {
                ::std::io::_print(format_args!("sex: {0}\n", sex));
            };
            let base_addr = ((((block.0 + HASHES_PER_HASH_TABLE) / HASHES_PER_HASH_TABLE)
                << sex) + block.0) as u64;
            if block.0 < HASHES_PER_HASH_TABLE {
                base_addr
            } else if block.0 < DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] {
                base_addr
                    + (((base_addr + DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] as u64)
                        / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] as u64) << sex)
            } else {
                ((1 << sex as usize)
                    + ((base_addr as usize
                        + ((block.0 + DATA_BLOCKS_PER_HASH_TREE_LEVEL[2])
                            / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2])) << sex as usize))
                    as u64
            }
        }
    }
    pub struct StfsFileEntryAddressingInfo {
        pub file_table_index: usize,
        pub file_entry_address: u64,
    }
    #[automatically_derived]
    impl ::core::default::Default for StfsFileEntryAddressingInfo {
        #[inline]
        fn default() -> StfsFileEntryAddressingInfo {
            StfsFileEntryAddressingInfo {
                file_table_index: ::core::default::Default::default(),
                file_entry_address: ::core::default::Default::default(),
            }
        }
    }
    #[automatically_derived]
    impl ::core::clone::Clone for StfsFileEntryAddressingInfo {
        #[inline]
        fn clone(&self) -> StfsFileEntryAddressingInfo {
            StfsFileEntryAddressingInfo {
                file_table_index: ::core::clone::Clone::clone(&self.file_table_index),
                file_entry_address: ::core::clone::Clone::clone(&self.file_entry_address),
            }
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for StfsFileEntryAddressingInfo {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field2_finish(
                f,
                "StfsFileEntryAddressingInfo",
                "file_table_index",
                &self.file_table_index,
                "file_entry_address",
                &&self.file_entry_address,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for StfsFileEntryAddressingInfo {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "StfsFileEntryAddressingInfo",
                    false as usize + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "file_table_index",
                    &self.file_table_index,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "file_entry_address",
                    &self.file_entry_address,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    #[allow(clippy::identity_op)]
    pub struct StfTimestamp {
        bytes: [::core::primitive::u8; {
            ((({
                0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                    + <B6 as ::modular_bitfield::Specifier>::BITS
                    + <B5 as ::modular_bitfield::Specifier>::BITS
                    + <B5 as ::modular_bitfield::Specifier>::BITS
                    + <B4 as ::modular_bitfield::Specifier>::BITS
                    + <B7 as ::modular_bitfield::Specifier>::BITS
            } - 1) / 8) + 1) * 8
        } / 8usize],
    }
    #[automatically_derived]
    #[allow(clippy::identity_op)]
    impl ::core::default::Default for StfTimestamp {
        #[inline]
        fn default() -> StfTimestamp {
            StfTimestamp {
                bytes: ::core::default::Default::default(),
            }
        }
    }
    #[automatically_derived]
    #[allow(clippy::identity_op)]
    impl ::core::marker::Copy for StfTimestamp {}
    #[automatically_derived]
    #[allow(clippy::identity_op)]
    impl ::core::clone::Clone for StfTimestamp {
        #[inline]
        fn clone(&self) -> StfTimestamp {
            let _: ::core::clone::AssertParamIsClone<
                [::core::primitive::u8; {
                    ((({
                        0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                            + <B6 as ::modular_bitfield::Specifier>::BITS
                            + <B5 as ::modular_bitfield::Specifier>::BITS
                            + <B5 as ::modular_bitfield::Specifier>::BITS
                            + <B4 as ::modular_bitfield::Specifier>::BITS
                            + <B7 as ::modular_bitfield::Specifier>::BITS
                    } - 1) / 8) + 1) * 8
                } / 8usize],
            >;
            *self
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for StfTimestamp {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "StfTimestamp",
                    false as usize + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "bytes",
                    &self.bytes,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    #[automatically_derived]
    #[allow(clippy::identity_op)]
    impl ::core::cmp::Eq for StfTimestamp {
        #[inline]
        #[doc(hidden)]
        #[coverage(off)]
        fn assert_receiver_is_total_eq(&self) -> () {
            let _: ::core::cmp::AssertParamIsEq<
                [::core::primitive::u8; {
                    ((({
                        0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                            + <B6 as ::modular_bitfield::Specifier>::BITS
                            + <B5 as ::modular_bitfield::Specifier>::BITS
                            + <B5 as ::modular_bitfield::Specifier>::BITS
                            + <B4 as ::modular_bitfield::Specifier>::BITS
                            + <B7 as ::modular_bitfield::Specifier>::BITS
                    } - 1) / 8) + 1) * 8
                } / 8usize],
            >;
        }
    }
    #[automatically_derived]
    #[allow(clippy::identity_op)]
    impl ::core::marker::StructuralPartialEq for StfTimestamp {}
    #[automatically_derived]
    #[allow(clippy::identity_op)]
    impl ::core::cmp::PartialEq for StfTimestamp {
        #[inline]
        fn eq(&self, other: &StfTimestamp) -> bool {
            self.bytes == other.bytes
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for StfTimestamp {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                binrw::BinRead::read_options(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        (),
                    )
                    .map(|x: u32| { Self::from(x) })
                    .and_then(|__binrw_this| {
                        let Self { ref bytes } = &__binrw_this;
                        (|| { Ok(()) })().map(|_: ()| __binrw_this)
                    })
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    impl binrw::meta::ReadEndian for StfTimestamp {
        const ENDIAN: binrw::meta::EndianKind = binrw::meta::EndianKind::None;
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for StfTimestamp {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            binrw::BinWrite::write_options(
                &((|ts: &Self| u32::from(ts))(self)),
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                (),
            )?;
            Ok(())
        }
    }
    impl binrw::meta::WriteEndian for StfTimestamp {
        const ENDIAN: binrw::meta::EndianKind = binrw::meta::EndianKind::None;
    }
    #[allow(clippy::identity_op)]
    const _: () = {
        impl ::modular_bitfield::private::checks::CheckTotalSizeMultipleOf8
        for StfTimestamp {
            type Size = ::modular_bitfield::private::checks::TotalSize<
                [(); {
                    0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                        + <B6 as ::modular_bitfield::Specifier>::BITS
                        + <B5 as ::modular_bitfield::Specifier>::BITS
                        + <B5 as ::modular_bitfield::Specifier>::BITS
                        + <B4 as ::modular_bitfield::Specifier>::BITS
                        + <B7 as ::modular_bitfield::Specifier>::BITS
                } % 8usize],
            >;
        }
    };
    impl StfTimestamp {
        /// Returns an instance with zero initialized data.
        #[allow(clippy::identity_op)]
        pub const fn new() -> Self {
            Self {
                bytes: [0u8; {
                    ((({
                        0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                            + <B6 as ::modular_bitfield::Specifier>::BITS
                            + <B5 as ::modular_bitfield::Specifier>::BITS
                            + <B5 as ::modular_bitfield::Specifier>::BITS
                            + <B4 as ::modular_bitfield::Specifier>::BITS
                            + <B7 as ::modular_bitfield::Specifier>::BITS
                    } - 1) / 8) + 1) * 8
                } / 8usize],
            }
        }
    }
    impl StfTimestamp {
        /// Returns the underlying bits.
        ///
        /// # Layout
        ///
        /// The returned byte array is layed out in the same way as described
        /// [here](https://docs.rs/modular-bitfield/#generated-structure).
        #[inline]
        #[allow(clippy::identity_op)]
        pub const fn into_bytes(
            self,
        ) -> [::core::primitive::u8; {
            ((({
                0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                    + <B6 as ::modular_bitfield::Specifier>::BITS
                    + <B5 as ::modular_bitfield::Specifier>::BITS
                    + <B5 as ::modular_bitfield::Specifier>::BITS
                    + <B4 as ::modular_bitfield::Specifier>::BITS
                    + <B7 as ::modular_bitfield::Specifier>::BITS
            } - 1) / 8) + 1) * 8
        } / 8usize] {
            self.bytes
        }
        /// Converts the given bytes directly into the bitfield struct.
        #[inline]
        #[allow(clippy::identity_op)]
        pub const fn from_bytes(
            bytes: [::core::primitive::u8; {
                ((({
                    0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                        + <B6 as ::modular_bitfield::Specifier>::BITS
                        + <B5 as ::modular_bitfield::Specifier>::BITS
                        + <B5 as ::modular_bitfield::Specifier>::BITS
                        + <B4 as ::modular_bitfield::Specifier>::BITS
                        + <B7 as ::modular_bitfield::Specifier>::BITS
                } - 1) / 8) + 1) * 8
            } / 8usize],
        ) -> Self {
            Self { bytes }
        }
    }
    const _: () = {
        const _: () = {};
        const _: () = {};
        const _: () = {};
        const _: () = {};
        const _: () = {};
        const _: () = {};
    };
    impl StfTimestamp {
        ///Returns the value of seconds.
        #[inline]
        pub fn seconds(&self) -> <B5 as ::modular_bitfield::Specifier>::InOut {
            self.seconds_or_err()
                .expect(
                    "value contains invalid bit pattern for field StfTimestamp.seconds",
                )
        }
        /**Returns the value of seconds.

#Errors

If the returned value contains an invalid bit pattern for seconds.*/
        #[inline]
        #[allow(dead_code)]
        pub fn seconds_or_err(
            &self,
        ) -> ::core::result::Result<
            <B5 as ::modular_bitfield::Specifier>::InOut,
            ::modular_bitfield::error::InvalidBitPattern<
                <B5 as ::modular_bitfield::Specifier>::Bytes,
            >,
        > {
            let __bf_read: <B5 as ::modular_bitfield::Specifier>::Bytes = {
                ::modular_bitfield::private::read_specifier::<
                    B5,
                >(&self.bytes[..], 0usize)
            };
            <B5 as ::modular_bitfield::Specifier>::from_bytes(__bf_read)
        }
        /**Returns a copy of the bitfield with the value of seconds set to the given value.

#Panics

If the given value is out of bounds for seconds.*/
        #[inline]
        #[allow(dead_code)]
        pub fn with_seconds(
            mut self,
            new_val: <B5 as ::modular_bitfield::Specifier>::InOut,
        ) -> Self {
            self.set_seconds(new_val);
            self
        }
        /**Returns a copy of the bitfield with the value of seconds set to the given value.

#Errors

If the given value is out of bounds for seconds.*/
        #[inline]
        #[allow(dead_code)]
        pub fn with_seconds_checked(
            mut self,
            new_val: <B5 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<Self, ::modular_bitfield::error::OutOfBounds> {
            self.set_seconds_checked(new_val)?;
            ::core::result::Result::Ok(self)
        }
        /**Sets the value of seconds to the given value.

#Panics

If the given value is out of bounds for seconds.*/
        #[inline]
        #[allow(dead_code)]
        pub fn set_seconds(
            &mut self,
            new_val: <B5 as ::modular_bitfield::Specifier>::InOut,
        ) {
            self.set_seconds_checked(new_val)
                .expect("value out of bounds for field StfTimestamp.seconds")
        }
        /**Sets the value of seconds to the given value.

#Errors

If the given value is out of bounds for seconds.*/
        #[inline]
        pub fn set_seconds_checked(
            &mut self,
            new_val: <B5 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<(), ::modular_bitfield::error::OutOfBounds> {
            let __bf_base_bits: ::core::primitive::usize = 8usize
                * ::core::mem::size_of::<<B5 as ::modular_bitfield::Specifier>::Bytes>();
            let __bf_max_value: <B5 as ::modular_bitfield::Specifier>::Bytes = {
                !0 >> (__bf_base_bits - <B5 as ::modular_bitfield::Specifier>::BITS)
            };
            let __bf_spec_bits: ::core::primitive::usize = <B5 as ::modular_bitfield::Specifier>::BITS;
            let __bf_raw_val: <B5 as ::modular_bitfield::Specifier>::Bytes = {
                <B5 as ::modular_bitfield::Specifier>::into_bytes(new_val)
            }?;
            if !(__bf_base_bits == __bf_spec_bits || __bf_raw_val <= __bf_max_value) {
                return ::core::result::Result::Err(
                    ::modular_bitfield::error::OutOfBounds,
                );
            }
            ::modular_bitfield::private::write_specifier::<
                B5,
            >(&mut self.bytes[..], 0usize, __bf_raw_val);
            ::core::result::Result::Ok(())
        }
        ///Returns the value of minute.
        #[inline]
        pub fn minute(&self) -> <B6 as ::modular_bitfield::Specifier>::InOut {
            self.minute_or_err()
                .expect(
                    "value contains invalid bit pattern for field StfTimestamp.minute",
                )
        }
        /**Returns the value of minute.

#Errors

If the returned value contains an invalid bit pattern for minute.*/
        #[inline]
        #[allow(dead_code)]
        pub fn minute_or_err(
            &self,
        ) -> ::core::result::Result<
            <B6 as ::modular_bitfield::Specifier>::InOut,
            ::modular_bitfield::error::InvalidBitPattern<
                <B6 as ::modular_bitfield::Specifier>::Bytes,
            >,
        > {
            let __bf_read: <B6 as ::modular_bitfield::Specifier>::Bytes = {
                ::modular_bitfield::private::read_specifier::<
                    B6,
                >(&self.bytes[..], 0usize + <B5 as ::modular_bitfield::Specifier>::BITS)
            };
            <B6 as ::modular_bitfield::Specifier>::from_bytes(__bf_read)
        }
        /**Returns a copy of the bitfield with the value of minute set to the given value.

#Panics

If the given value is out of bounds for minute.*/
        #[inline]
        #[allow(dead_code)]
        pub fn with_minute(
            mut self,
            new_val: <B6 as ::modular_bitfield::Specifier>::InOut,
        ) -> Self {
            self.set_minute(new_val);
            self
        }
        /**Returns a copy of the bitfield with the value of minute set to the given value.

#Errors

If the given value is out of bounds for minute.*/
        #[inline]
        #[allow(dead_code)]
        pub fn with_minute_checked(
            mut self,
            new_val: <B6 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<Self, ::modular_bitfield::error::OutOfBounds> {
            self.set_minute_checked(new_val)?;
            ::core::result::Result::Ok(self)
        }
        /**Sets the value of minute to the given value.

#Panics

If the given value is out of bounds for minute.*/
        #[inline]
        #[allow(dead_code)]
        pub fn set_minute(
            &mut self,
            new_val: <B6 as ::modular_bitfield::Specifier>::InOut,
        ) {
            self.set_minute_checked(new_val)
                .expect("value out of bounds for field StfTimestamp.minute")
        }
        /**Sets the value of minute to the given value.

#Errors

If the given value is out of bounds for minute.*/
        #[inline]
        pub fn set_minute_checked(
            &mut self,
            new_val: <B6 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<(), ::modular_bitfield::error::OutOfBounds> {
            let __bf_base_bits: ::core::primitive::usize = 8usize
                * ::core::mem::size_of::<<B6 as ::modular_bitfield::Specifier>::Bytes>();
            let __bf_max_value: <B6 as ::modular_bitfield::Specifier>::Bytes = {
                !0 >> (__bf_base_bits - <B6 as ::modular_bitfield::Specifier>::BITS)
            };
            let __bf_spec_bits: ::core::primitive::usize = <B6 as ::modular_bitfield::Specifier>::BITS;
            let __bf_raw_val: <B6 as ::modular_bitfield::Specifier>::Bytes = {
                <B6 as ::modular_bitfield::Specifier>::into_bytes(new_val)
            }?;
            if !(__bf_base_bits == __bf_spec_bits || __bf_raw_val <= __bf_max_value) {
                return ::core::result::Result::Err(
                    ::modular_bitfield::error::OutOfBounds,
                );
            }
            ::modular_bitfield::private::write_specifier::<
                B6,
            >(
                &mut self.bytes[..],
                0usize + <B5 as ::modular_bitfield::Specifier>::BITS,
                __bf_raw_val,
            );
            ::core::result::Result::Ok(())
        }
        ///Returns the value of hour.
        #[inline]
        pub fn hour(&self) -> <B5 as ::modular_bitfield::Specifier>::InOut {
            self.hour_or_err()
                .expect("value contains invalid bit pattern for field StfTimestamp.hour")
        }
        /**Returns the value of hour.

#Errors

If the returned value contains an invalid bit pattern for hour.*/
        #[inline]
        #[allow(dead_code)]
        pub fn hour_or_err(
            &self,
        ) -> ::core::result::Result<
            <B5 as ::modular_bitfield::Specifier>::InOut,
            ::modular_bitfield::error::InvalidBitPattern<
                <B5 as ::modular_bitfield::Specifier>::Bytes,
            >,
        > {
            let __bf_read: <B5 as ::modular_bitfield::Specifier>::Bytes = {
                ::modular_bitfield::private::read_specifier::<
                    B5,
                >(
                    &self.bytes[..],
                    0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                        + <B6 as ::modular_bitfield::Specifier>::BITS,
                )
            };
            <B5 as ::modular_bitfield::Specifier>::from_bytes(__bf_read)
        }
        /**Returns a copy of the bitfield with the value of hour set to the given value.

#Panics

If the given value is out of bounds for hour.*/
        #[inline]
        #[allow(dead_code)]
        pub fn with_hour(
            mut self,
            new_val: <B5 as ::modular_bitfield::Specifier>::InOut,
        ) -> Self {
            self.set_hour(new_val);
            self
        }
        /**Returns a copy of the bitfield with the value of hour set to the given value.

#Errors

If the given value is out of bounds for hour.*/
        #[inline]
        #[allow(dead_code)]
        pub fn with_hour_checked(
            mut self,
            new_val: <B5 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<Self, ::modular_bitfield::error::OutOfBounds> {
            self.set_hour_checked(new_val)?;
            ::core::result::Result::Ok(self)
        }
        /**Sets the value of hour to the given value.

#Panics

If the given value is out of bounds for hour.*/
        #[inline]
        #[allow(dead_code)]
        pub fn set_hour(
            &mut self,
            new_val: <B5 as ::modular_bitfield::Specifier>::InOut,
        ) {
            self.set_hour_checked(new_val)
                .expect("value out of bounds for field StfTimestamp.hour")
        }
        /**Sets the value of hour to the given value.

#Errors

If the given value is out of bounds for hour.*/
        #[inline]
        pub fn set_hour_checked(
            &mut self,
            new_val: <B5 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<(), ::modular_bitfield::error::OutOfBounds> {
            let __bf_base_bits: ::core::primitive::usize = 8usize
                * ::core::mem::size_of::<<B5 as ::modular_bitfield::Specifier>::Bytes>();
            let __bf_max_value: <B5 as ::modular_bitfield::Specifier>::Bytes = {
                !0 >> (__bf_base_bits - <B5 as ::modular_bitfield::Specifier>::BITS)
            };
            let __bf_spec_bits: ::core::primitive::usize = <B5 as ::modular_bitfield::Specifier>::BITS;
            let __bf_raw_val: <B5 as ::modular_bitfield::Specifier>::Bytes = {
                <B5 as ::modular_bitfield::Specifier>::into_bytes(new_val)
            }?;
            if !(__bf_base_bits == __bf_spec_bits || __bf_raw_val <= __bf_max_value) {
                return ::core::result::Result::Err(
                    ::modular_bitfield::error::OutOfBounds,
                );
            }
            ::modular_bitfield::private::write_specifier::<
                B5,
            >(
                &mut self.bytes[..],
                0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                    + <B6 as ::modular_bitfield::Specifier>::BITS,
                __bf_raw_val,
            );
            ::core::result::Result::Ok(())
        }
        ///Returns the value of day.
        #[inline]
        pub fn day(&self) -> <B5 as ::modular_bitfield::Specifier>::InOut {
            self.day_or_err()
                .expect("value contains invalid bit pattern for field StfTimestamp.day")
        }
        /**Returns the value of day.

#Errors

If the returned value contains an invalid bit pattern for day.*/
        #[inline]
        #[allow(dead_code)]
        pub fn day_or_err(
            &self,
        ) -> ::core::result::Result<
            <B5 as ::modular_bitfield::Specifier>::InOut,
            ::modular_bitfield::error::InvalidBitPattern<
                <B5 as ::modular_bitfield::Specifier>::Bytes,
            >,
        > {
            let __bf_read: <B5 as ::modular_bitfield::Specifier>::Bytes = {
                ::modular_bitfield::private::read_specifier::<
                    B5,
                >(
                    &self.bytes[..],
                    0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                        + <B6 as ::modular_bitfield::Specifier>::BITS
                        + <B5 as ::modular_bitfield::Specifier>::BITS,
                )
            };
            <B5 as ::modular_bitfield::Specifier>::from_bytes(__bf_read)
        }
        /**Returns a copy of the bitfield with the value of day set to the given value.

#Panics

If the given value is out of bounds for day.*/
        #[inline]
        #[allow(dead_code)]
        pub fn with_day(
            mut self,
            new_val: <B5 as ::modular_bitfield::Specifier>::InOut,
        ) -> Self {
            self.set_day(new_val);
            self
        }
        /**Returns a copy of the bitfield with the value of day set to the given value.

#Errors

If the given value is out of bounds for day.*/
        #[inline]
        #[allow(dead_code)]
        pub fn with_day_checked(
            mut self,
            new_val: <B5 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<Self, ::modular_bitfield::error::OutOfBounds> {
            self.set_day_checked(new_val)?;
            ::core::result::Result::Ok(self)
        }
        /**Sets the value of day to the given value.

#Panics

If the given value is out of bounds for day.*/
        #[inline]
        #[allow(dead_code)]
        pub fn set_day(
            &mut self,
            new_val: <B5 as ::modular_bitfield::Specifier>::InOut,
        ) {
            self.set_day_checked(new_val)
                .expect("value out of bounds for field StfTimestamp.day")
        }
        /**Sets the value of day to the given value.

#Errors

If the given value is out of bounds for day.*/
        #[inline]
        pub fn set_day_checked(
            &mut self,
            new_val: <B5 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<(), ::modular_bitfield::error::OutOfBounds> {
            let __bf_base_bits: ::core::primitive::usize = 8usize
                * ::core::mem::size_of::<<B5 as ::modular_bitfield::Specifier>::Bytes>();
            let __bf_max_value: <B5 as ::modular_bitfield::Specifier>::Bytes = {
                !0 >> (__bf_base_bits - <B5 as ::modular_bitfield::Specifier>::BITS)
            };
            let __bf_spec_bits: ::core::primitive::usize = <B5 as ::modular_bitfield::Specifier>::BITS;
            let __bf_raw_val: <B5 as ::modular_bitfield::Specifier>::Bytes = {
                <B5 as ::modular_bitfield::Specifier>::into_bytes(new_val)
            }?;
            if !(__bf_base_bits == __bf_spec_bits || __bf_raw_val <= __bf_max_value) {
                return ::core::result::Result::Err(
                    ::modular_bitfield::error::OutOfBounds,
                );
            }
            ::modular_bitfield::private::write_specifier::<
                B5,
            >(
                &mut self.bytes[..],
                0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                    + <B6 as ::modular_bitfield::Specifier>::BITS
                    + <B5 as ::modular_bitfield::Specifier>::BITS,
                __bf_raw_val,
            );
            ::core::result::Result::Ok(())
        }
        ///Returns the value of month.
        #[inline]
        pub fn month(&self) -> <B4 as ::modular_bitfield::Specifier>::InOut {
            self.month_or_err()
                .expect(
                    "value contains invalid bit pattern for field StfTimestamp.month",
                )
        }
        /**Returns the value of month.

#Errors

If the returned value contains an invalid bit pattern for month.*/
        #[inline]
        #[allow(dead_code)]
        pub fn month_or_err(
            &self,
        ) -> ::core::result::Result<
            <B4 as ::modular_bitfield::Specifier>::InOut,
            ::modular_bitfield::error::InvalidBitPattern<
                <B4 as ::modular_bitfield::Specifier>::Bytes,
            >,
        > {
            let __bf_read: <B4 as ::modular_bitfield::Specifier>::Bytes = {
                ::modular_bitfield::private::read_specifier::<
                    B4,
                >(
                    &self.bytes[..],
                    0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                        + <B6 as ::modular_bitfield::Specifier>::BITS
                        + <B5 as ::modular_bitfield::Specifier>::BITS
                        + <B5 as ::modular_bitfield::Specifier>::BITS,
                )
            };
            <B4 as ::modular_bitfield::Specifier>::from_bytes(__bf_read)
        }
        /**Returns a copy of the bitfield with the value of month set to the given value.

#Panics

If the given value is out of bounds for month.*/
        #[inline]
        #[allow(dead_code)]
        pub fn with_month(
            mut self,
            new_val: <B4 as ::modular_bitfield::Specifier>::InOut,
        ) -> Self {
            self.set_month(new_val);
            self
        }
        /**Returns a copy of the bitfield with the value of month set to the given value.

#Errors

If the given value is out of bounds for month.*/
        #[inline]
        #[allow(dead_code)]
        pub fn with_month_checked(
            mut self,
            new_val: <B4 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<Self, ::modular_bitfield::error::OutOfBounds> {
            self.set_month_checked(new_val)?;
            ::core::result::Result::Ok(self)
        }
        /**Sets the value of month to the given value.

#Panics

If the given value is out of bounds for month.*/
        #[inline]
        #[allow(dead_code)]
        pub fn set_month(
            &mut self,
            new_val: <B4 as ::modular_bitfield::Specifier>::InOut,
        ) {
            self.set_month_checked(new_val)
                .expect("value out of bounds for field StfTimestamp.month")
        }
        /**Sets the value of month to the given value.

#Errors

If the given value is out of bounds for month.*/
        #[inline]
        pub fn set_month_checked(
            &mut self,
            new_val: <B4 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<(), ::modular_bitfield::error::OutOfBounds> {
            let __bf_base_bits: ::core::primitive::usize = 8usize
                * ::core::mem::size_of::<<B4 as ::modular_bitfield::Specifier>::Bytes>();
            let __bf_max_value: <B4 as ::modular_bitfield::Specifier>::Bytes = {
                !0 >> (__bf_base_bits - <B4 as ::modular_bitfield::Specifier>::BITS)
            };
            let __bf_spec_bits: ::core::primitive::usize = <B4 as ::modular_bitfield::Specifier>::BITS;
            let __bf_raw_val: <B4 as ::modular_bitfield::Specifier>::Bytes = {
                <B4 as ::modular_bitfield::Specifier>::into_bytes(new_val)
            }?;
            if !(__bf_base_bits == __bf_spec_bits || __bf_raw_val <= __bf_max_value) {
                return ::core::result::Result::Err(
                    ::modular_bitfield::error::OutOfBounds,
                );
            }
            ::modular_bitfield::private::write_specifier::<
                B4,
            >(
                &mut self.bytes[..],
                0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                    + <B6 as ::modular_bitfield::Specifier>::BITS
                    + <B5 as ::modular_bitfield::Specifier>::BITS
                    + <B5 as ::modular_bitfield::Specifier>::BITS,
                __bf_raw_val,
            );
            ::core::result::Result::Ok(())
        }
        ///Returns the value of year.
        #[inline]
        pub fn year(&self) -> <B7 as ::modular_bitfield::Specifier>::InOut {
            self.year_or_err()
                .expect("value contains invalid bit pattern for field StfTimestamp.year")
        }
        /**Returns the value of year.

#Errors

If the returned value contains an invalid bit pattern for year.*/
        #[inline]
        #[allow(dead_code)]
        pub fn year_or_err(
            &self,
        ) -> ::core::result::Result<
            <B7 as ::modular_bitfield::Specifier>::InOut,
            ::modular_bitfield::error::InvalidBitPattern<
                <B7 as ::modular_bitfield::Specifier>::Bytes,
            >,
        > {
            let __bf_read: <B7 as ::modular_bitfield::Specifier>::Bytes = {
                ::modular_bitfield::private::read_specifier::<
                    B7,
                >(
                    &self.bytes[..],
                    0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                        + <B6 as ::modular_bitfield::Specifier>::BITS
                        + <B5 as ::modular_bitfield::Specifier>::BITS
                        + <B5 as ::modular_bitfield::Specifier>::BITS
                        + <B4 as ::modular_bitfield::Specifier>::BITS,
                )
            };
            <B7 as ::modular_bitfield::Specifier>::from_bytes(__bf_read)
        }
        /**Returns a copy of the bitfield with the value of year set to the given value.

#Panics

If the given value is out of bounds for year.*/
        #[inline]
        #[allow(dead_code)]
        pub fn with_year(
            mut self,
            new_val: <B7 as ::modular_bitfield::Specifier>::InOut,
        ) -> Self {
            self.set_year(new_val);
            self
        }
        /**Returns a copy of the bitfield with the value of year set to the given value.

#Errors

If the given value is out of bounds for year.*/
        #[inline]
        #[allow(dead_code)]
        pub fn with_year_checked(
            mut self,
            new_val: <B7 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<Self, ::modular_bitfield::error::OutOfBounds> {
            self.set_year_checked(new_val)?;
            ::core::result::Result::Ok(self)
        }
        /**Sets the value of year to the given value.

#Panics

If the given value is out of bounds for year.*/
        #[inline]
        #[allow(dead_code)]
        pub fn set_year(
            &mut self,
            new_val: <B7 as ::modular_bitfield::Specifier>::InOut,
        ) {
            self.set_year_checked(new_val)
                .expect("value out of bounds for field StfTimestamp.year")
        }
        /**Sets the value of year to the given value.

#Errors

If the given value is out of bounds for year.*/
        #[inline]
        pub fn set_year_checked(
            &mut self,
            new_val: <B7 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<(), ::modular_bitfield::error::OutOfBounds> {
            let __bf_base_bits: ::core::primitive::usize = 8usize
                * ::core::mem::size_of::<<B7 as ::modular_bitfield::Specifier>::Bytes>();
            let __bf_max_value: <B7 as ::modular_bitfield::Specifier>::Bytes = {
                !0 >> (__bf_base_bits - <B7 as ::modular_bitfield::Specifier>::BITS)
            };
            let __bf_spec_bits: ::core::primitive::usize = <B7 as ::modular_bitfield::Specifier>::BITS;
            let __bf_raw_val: <B7 as ::modular_bitfield::Specifier>::Bytes = {
                <B7 as ::modular_bitfield::Specifier>::into_bytes(new_val)
            }?;
            if !(__bf_base_bits == __bf_spec_bits || __bf_raw_val <= __bf_max_value) {
                return ::core::result::Result::Err(
                    ::modular_bitfield::error::OutOfBounds,
                );
            }
            ::modular_bitfield::private::write_specifier::<
                B7,
            >(
                &mut self.bytes[..],
                0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                    + <B6 as ::modular_bitfield::Specifier>::BITS
                    + <B5 as ::modular_bitfield::Specifier>::BITS
                    + <B5 as ::modular_bitfield::Specifier>::BITS
                    + <B4 as ::modular_bitfield::Specifier>::BITS,
                __bf_raw_val,
            );
            ::core::result::Result::Ok(())
        }
    }
    impl ::core::convert::From<::core::primitive::u32> for StfTimestamp
    where
        [(); {
            0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                + <B6 as ::modular_bitfield::Specifier>::BITS
                + <B5 as ::modular_bitfield::Specifier>::BITS
                + <B5 as ::modular_bitfield::Specifier>::BITS
                + <B4 as ::modular_bitfield::Specifier>::BITS
                + <B7 as ::modular_bitfield::Specifier>::BITS
        }]: ::modular_bitfield::private::IsU32Compatible,
    {
        #[inline]
        fn from(__bf_prim: ::core::primitive::u32) -> Self {
            Self {
                bytes: <::core::primitive::u32>::to_le_bytes(__bf_prim),
            }
        }
    }
    impl ::core::convert::From<StfTimestamp> for ::core::primitive::u32
    where
        [(); {
            0usize + <B5 as ::modular_bitfield::Specifier>::BITS
                + <B6 as ::modular_bitfield::Specifier>::BITS
                + <B5 as ::modular_bitfield::Specifier>::BITS
                + <B5 as ::modular_bitfield::Specifier>::BITS
                + <B4 as ::modular_bitfield::Specifier>::BITS
                + <B7 as ::modular_bitfield::Specifier>::BITS
        }]: ::modular_bitfield::private::IsU32Compatible,
    {
        #[inline]
        fn from(__bf_bitfield: StfTimestamp) -> Self {
            <Self>::from_le_bytes(__bf_bitfield.bytes)
        }
    }
    impl ::core::fmt::Debug for StfTimestamp {
        fn fmt(&self, __bf_f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
            __bf_f
                .debug_struct("StfTimestamp")
                .field(
                    "seconds",
                    self
                        .seconds_or_err()
                        .as_ref()
                        .map(|__bf_field| __bf_field as &dyn ::core::fmt::Debug)
                        .unwrap_or_else(|__bf_err| __bf_err as &dyn ::core::fmt::Debug),
                )
                .field(
                    "minute",
                    self
                        .minute_or_err()
                        .as_ref()
                        .map(|__bf_field| __bf_field as &dyn ::core::fmt::Debug)
                        .unwrap_or_else(|__bf_err| __bf_err as &dyn ::core::fmt::Debug),
                )
                .field(
                    "hour",
                    self
                        .hour_or_err()
                        .as_ref()
                        .map(|__bf_field| __bf_field as &dyn ::core::fmt::Debug)
                        .unwrap_or_else(|__bf_err| __bf_err as &dyn ::core::fmt::Debug),
                )
                .field(
                    "day",
                    self
                        .day_or_err()
                        .as_ref()
                        .map(|__bf_field| __bf_field as &dyn ::core::fmt::Debug)
                        .unwrap_or_else(|__bf_err| __bf_err as &dyn ::core::fmt::Debug),
                )
                .field(
                    "month",
                    self
                        .month_or_err()
                        .as_ref()
                        .map(|__bf_field| __bf_field as &dyn ::core::fmt::Debug)
                        .unwrap_or_else(|__bf_err| __bf_err as &dyn ::core::fmt::Debug),
                )
                .field(
                    "year",
                    self
                        .year_or_err()
                        .as_ref()
                        .map(|__bf_field| __bf_field as &dyn ::core::fmt::Debug)
                        .unwrap_or_else(|__bf_err| __bf_err as &dyn ::core::fmt::Debug),
                )
                .finish()
        }
    }
    pub struct StfsFileAttributes {
        pub block_count: u32,
        pub allocation_block_count: u32,
        pub starting_block: Block,
        pub dirent: u16,
        pub file_size: u32,
        pub created_time_stamp: StfTimestamp,
        pub access_time_stamp: StfTimestamp,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for StfsFileAttributes {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_generated_read_function = binrw::__private::parse_fn_type_hint(
                    binrw::helpers::read_u24,
                );
                let __binrw_generated_endian_block_count = binrw::Endian::Little;
                let mut block_count: u32 = (|| {
                    __binrw_generated_read_function
                })()(
                        __binrw_generated_var_reader,
                        __binrw_generated_endian_block_count,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map(|v| -> u32 { v })
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'block_count' in StfsFileAttributes"
                                .into(),
                            line: 552u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   549 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mparse_with\u{1b}[39m = \u{1b}[38;5;197mbinrw\u{1b}[39m::helpers::read_u24\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   550 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39mwrite_with = binrw::helpers::write_u24 \u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   551 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mlittle\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m552\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub block_count: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::__private::parse_fn_type_hint(
                    binrw::helpers::read_u24,
                );
                let __binrw_generated_endian_allocation_block_count = binrw::Endian::Little;
                let mut allocation_block_count: u32 = (|| {
                    __binrw_generated_read_function
                })()(
                        __binrw_generated_var_reader,
                        __binrw_generated_endian_allocation_block_count,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map(|v| -> u32 { v })
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'allocation_block_count' in StfsFileAttributes"
                                .into(),
                            line: 557u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   554 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mparse_with\u{1b}[39m = \u{1b}[38;5;197mbinrw\u{1b}[39m::helpers::read_u24\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   555 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39mwrite_with = binrw::helpers::write_u24 \u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   556 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mlittle\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m557\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub allocation_block_count: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let __binrw_generated_endian_starting_block = binrw::Endian::Little;
                let mut starting_block: Block = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_endian_starting_block,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'starting_block' in StfsFileAttributes"
                                .into(),
                            line: 560u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   559 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mlittle\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m560\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub starting_block: Block\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut dirent: u16 = {
                    let __binrw_generated_saved_position = binrw::io::Seek::stream_position(
                        __binrw_generated_var_reader,
                    )?;
                    let __binrw_temp = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'dirent' in StfsFileAttributes"
                                    .into(),
                                line: 563u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   562 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mdbg\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m563\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub dirent: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu16\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    {
                        ::std::io::_eprint(
                            format_args!(
                                "[{0}:{1} | offset {2:#x}] {3} = {4:#x?}\n",
                                "stfs/src/parse.rs",
                                563usize,
                                __binrw_generated_saved_position,
                                "dirent",
                                &__binrw_temp,
                            ),
                        );
                    };
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut file_size: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'file_size' in StfsFileAttributes"
                                .into(),
                            line: 564u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m564\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub file_size: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut created_time_stamp: StfTimestamp = {
                    let __binrw_generated_saved_position = binrw::io::Seek::stream_position(
                        __binrw_generated_var_reader,
                    )?;
                    let __binrw_temp = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'created_time_stamp' in StfsFileAttributes"
                                    .into(),
                                line: 566u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   565 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mdbg\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m566\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub created_time_stamp: StfTimestamp\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    {
                        ::std::io::_eprint(
                            format_args!(
                                "[{0}:{1} | offset {2:#x}] {3} = {4:#x?}\n",
                                "stfs/src/parse.rs",
                                566usize,
                                __binrw_generated_saved_position,
                                "created_time_stamp",
                                &__binrw_temp,
                            ),
                        );
                    };
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut access_time_stamp: StfTimestamp = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'access_time_stamp' in StfsFileAttributes"
                                .into(),
                            line: 567u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m567\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub access_time_stamp: StfTimestamp\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_this = Self {
                    block_count,
                    allocation_block_count,
                    starting_block,
                    dirent,
                    file_size,
                    created_time_stamp,
                    access_time_stamp,
                };
                Ok(__binrw_this)
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for StfsFileAttributes {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_this = self;
            let StfsFileAttributes {
                ref block_count,
                ref allocation_block_count,
                ref starting_block,
                ref dirent,
                ref file_size,
                ref created_time_stamp,
                ref access_time_stamp,
            } = self;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::helpers::write_u24,
            );
            let __binrw_generated_args_block_count = binrw::__private::write_function_args_type_hint::<
                u32,
                _,
                _,
                _,
            >(
                __binrw_generated_write_function,
                <_ as binrw::__private::Required>::args(),
            );
            __binrw_generated_write_function(
                &block_count,
                __binrw_generated_var_writer,
                binrw::Endian::Little,
                __binrw_generated_args_block_count,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::helpers::write_u24,
            );
            let __binrw_generated_args_allocation_block_count = binrw::__private::write_function_args_type_hint::<
                u32,
                _,
                _,
                _,
            >(
                __binrw_generated_write_function,
                <_ as binrw::__private::Required>::args(),
            );
            __binrw_generated_write_function(
                &allocation_block_count,
                __binrw_generated_var_writer,
                binrw::Endian::Little,
                __binrw_generated_args_allocation_block_count,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_starting_block: <Block as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &starting_block,
                __binrw_generated_var_writer,
                binrw::Endian::Little,
                __binrw_generated_args_starting_block,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_dirent: <u16 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &dirent,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_dirent,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_file_size: <u32 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &file_size,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_file_size,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_created_time_stamp: <StfTimestamp as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &created_time_stamp,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_created_time_stamp,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_access_time_stamp: <StfTimestamp as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &access_time_stamp,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_access_time_stamp,
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::default::Default for StfsFileAttributes {
        #[inline]
        fn default() -> StfsFileAttributes {
            StfsFileAttributes {
                block_count: ::core::default::Default::default(),
                allocation_block_count: ::core::default::Default::default(),
                starting_block: ::core::default::Default::default(),
                dirent: ::core::default::Default::default(),
                file_size: ::core::default::Default::default(),
                created_time_stamp: ::core::default::Default::default(),
                access_time_stamp: ::core::default::Default::default(),
            }
        }
    }
    #[automatically_derived]
    impl ::core::clone::Clone for StfsFileAttributes {
        #[inline]
        fn clone(&self) -> StfsFileAttributes {
            StfsFileAttributes {
                block_count: ::core::clone::Clone::clone(&self.block_count),
                allocation_block_count: ::core::clone::Clone::clone(
                    &self.allocation_block_count,
                ),
                starting_block: ::core::clone::Clone::clone(&self.starting_block),
                dirent: ::core::clone::Clone::clone(&self.dirent),
                file_size: ::core::clone::Clone::clone(&self.file_size),
                created_time_stamp: ::core::clone::Clone::clone(
                    &self.created_time_stamp,
                ),
                access_time_stamp: ::core::clone::Clone::clone(&self.access_time_stamp),
            }
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for StfsFileAttributes {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            let names: &'static _ = &[
                "block_count",
                "allocation_block_count",
                "starting_block",
                "dirent",
                "file_size",
                "created_time_stamp",
                "access_time_stamp",
            ];
            let values: &[&dyn ::core::fmt::Debug] = &[
                &self.block_count,
                &self.allocation_block_count,
                &self.starting_block,
                &self.dirent,
                &self.file_size,
                &self.created_time_stamp,
                &&self.access_time_stamp,
            ];
            ::core::fmt::Formatter::debug_struct_fields_finish(
                f,
                "StfsFileAttributes",
                names,
                values,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for StfsFileAttributes {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "StfsFileAttributes",
                    false as usize + 1 + 1 + 1 + 1 + 1 + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "block_count",
                    &self.block_count,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "allocation_block_count",
                    &self.allocation_block_count,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "starting_block",
                    &self.starting_block,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "dirent",
                    &self.dirent,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "file_size",
                    &self.file_size,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "created_time_stamp",
                    &self.created_time_stamp,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "access_time_stamp",
                    &self.access_time_stamp,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    pub struct StfsFileEntry {
        pub addressing_info: StfsFileEntryAddressingInfo,
        #[serde(serialize_with = "serialize_null_string")]
        pub name: NullString,
        pub flags: StfsEntryFlags,
        pub file_attributes: Option<StfsFileAttributes>,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for StfsFileEntry {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let mut addressing_info: StfsFileEntryAddressingInfo = <_>::default();
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut name: NullString = {
                    let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                        __binrw_generated_var_reader,
                    )?;
                    let __binrw_temp = {
                        let __binrw_generated_saved_position = binrw::io::Seek::stream_position(
                            __binrw_generated_var_reader,
                        )?;
                        let __binrw_temp = __binrw_generated_read_function(
                                __binrw_generated_var_reader,
                                __binrw_generated_var_endian,
                                <_ as binrw::__private::Required>::args(),
                            )
                            .map_err(|err| binrw::error::ContextExt::with_context(
                                err,
                                binrw::error::BacktraceFrame::Full {
                                    message: "While parsing field 'name' in StfsFileEntry"
                                        .into(),
                                    line: 579u32,
                                    file: "stfs/src/parse.rs",
                                    code: Some(
                                        "  ┄────╮\n   576 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mpad_size_to\u{1b}[39m = \u{1b}[38;5;135m0x28\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   577 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mserde\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39mserialize_with = \"serialize_null_string\"\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   578 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mdbg\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m579\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub name: NullString\u{1b}[0m\n  ┄────╯\n",
                                    ),
                                },
                            ))?;
                        {
                            ::std::io::_eprint(
                                format_args!(
                                    "[{0}:{1} | offset {2:#x}] {3} = {4:#x?}\n",
                                    "stfs/src/parse.rs",
                                    579usize,
                                    __binrw_generated_saved_position,
                                    "name",
                                    &__binrw_temp,
                                ),
                            );
                        };
                        {
                            {
                                ::std::io::_eprint(
                                    format_args!(
                                        "[{0}:{1} | pad_size_to {2:#x}]\n",
                                        "stfs/src/parse.rs",
                                        579usize,
                                        0x28,
                                    ),
                                );
                            };
                        }
                        __binrw_temp
                    };
                    {
                        let pad = (0x28) as i64;
                        let size = (binrw::io::Seek::stream_position(
                            __binrw_generated_var_reader,
                        )? - __binrw_generated_position_temp) as i64;
                        if size < pad {
                            binrw::io::Seek::seek(
                                __binrw_generated_var_reader,
                                binrw::io::SeekFrom::Current(pad - size),
                            )?;
                        }
                    }
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut flags: StfsEntryFlags = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'flags' in StfsFileEntry"
                                .into(),
                            line: 580u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m580\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub flags: StfsEntryFlags\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut file_attributes: Option<StfsFileAttributes> = if flags.name_len()
                    > 0
                {
                    __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'file_attributes' in StfsFileEntry"
                                    .into(),
                                line: 583u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   582 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mif\u{1b}[39m(flags.\u{1b}[38;5;148mname_len\u{1b}[39m() \u{1b}[38;5;197m>\u{1b}[39m \u{1b}[38;5;135m0\u{1b}[39m)\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m583\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub file_attributes: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mOption\u{1b}[39m\u{1b}[0m\u{1b}[1m<StfsFileAttributes>\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?
                } else {
                    <_>::default()
                };
                let __binrw_this = Self {
                    addressing_info,
                    name,
                    flags,
                    file_attributes,
                };
                Ok(__binrw_this)
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for StfsFileEntry {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_this = self;
            let StfsFileEntry {
                ref addressing_info,
                ref name,
                ref flags,
                ref file_attributes,
            } = self;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_name: <NullString as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            let __binrw_generated_before_pos = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            __binrw_generated_write_function(
                &name,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_name,
            )?;
            {
                let pad_to_size = (0x28) as u64;
                let after_pos = binrw::io::Seek::stream_position(
                    __binrw_generated_var_writer,
                )?;
                if let Some(size) = after_pos.checked_sub(__binrw_generated_before_pos) {
                    if let Some(padding) = pad_to_size.checked_sub(size) {
                        binrw::__private::write_zeroes(
                            __binrw_generated_var_writer,
                            padding,
                        )?;
                    }
                }
            }
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_flags: <StfsEntryFlags as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &flags,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_flags,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_file_attributes: <Option<
                StfsFileAttributes,
            > as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &file_attributes,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_file_attributes,
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::default::Default for StfsFileEntry {
        #[inline]
        fn default() -> StfsFileEntry {
            StfsFileEntry {
                addressing_info: ::core::default::Default::default(),
                name: ::core::default::Default::default(),
                flags: ::core::default::Default::default(),
                file_attributes: ::core::default::Default::default(),
            }
        }
    }
    #[automatically_derived]
    impl ::core::clone::Clone for StfsFileEntry {
        #[inline]
        fn clone(&self) -> StfsFileEntry {
            StfsFileEntry {
                addressing_info: ::core::clone::Clone::clone(&self.addressing_info),
                name: ::core::clone::Clone::clone(&self.name),
                flags: ::core::clone::Clone::clone(&self.flags),
                file_attributes: ::core::clone::Clone::clone(&self.file_attributes),
            }
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for StfsFileEntry {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field4_finish(
                f,
                "StfsFileEntry",
                "addressing_info",
                &self.addressing_info,
                "name",
                &self.name,
                "flags",
                &self.flags,
                "file_attributes",
                &&self.file_attributes,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for StfsFileEntry {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "StfsFileEntry",
                    false as usize + 1 + 1 + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "addressing_info",
                    &self.addressing_info,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "name",
                    {
                        #[doc(hidden)]
                        struct __SerializeWith<'__a> {
                            values: (&'__a NullString,),
                            phantom: _serde::__private::PhantomData<StfsFileEntry>,
                        }
                        impl<'__a> _serde::Serialize for __SerializeWith<'__a> {
                            fn serialize<__S>(
                                &self,
                                __s: __S,
                            ) -> _serde::__private::Result<__S::Ok, __S::Error>
                            where
                                __S: _serde::Serializer,
                            {
                                serialize_null_string(self.values.0, __s)
                            }
                        }
                        &__SerializeWith {
                            values: (&self.name,),
                            phantom: _serde::__private::PhantomData::<StfsFileEntry>,
                        }
                    },
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "flags",
                    &self.flags,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "file_attributes",
                    &self.file_attributes,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    #[allow(clippy::identity_op)]
    pub struct StfsEntryFlags {
        bytes: [::core::primitive::u8; {
            ((({
                0usize + <B6 as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS
            } - 1) / 8) + 1) * 8
        } / 8usize],
    }
    #[automatically_derived]
    #[allow(clippy::identity_op)]
    impl ::core::default::Default for StfsEntryFlags {
        #[inline]
        fn default() -> StfsEntryFlags {
            StfsEntryFlags {
                bytes: ::core::default::Default::default(),
            }
        }
    }
    #[automatically_derived]
    #[allow(clippy::identity_op)]
    impl ::core::marker::Copy for StfsEntryFlags {}
    #[automatically_derived]
    #[allow(clippy::identity_op)]
    impl ::core::clone::Clone for StfsEntryFlags {
        #[inline]
        fn clone(&self) -> StfsEntryFlags {
            let _: ::core::clone::AssertParamIsClone<
                [::core::primitive::u8; {
                    ((({
                        0usize + <B6 as ::modular_bitfield::Specifier>::BITS
                            + <bool as ::modular_bitfield::Specifier>::BITS
                            + <bool as ::modular_bitfield::Specifier>::BITS
                    } - 1) / 8) + 1) * 8
                } / 8usize],
            >;
            *self
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for StfsEntryFlags {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "StfsEntryFlags",
                    false as usize + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "bytes",
                    &self.bytes,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for StfsEntryFlags {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                binrw::BinRead::read_options(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        (),
                    )
                    .map(Self::from_bytes)
                    .and_then(|__binrw_this| {
                        let Self { ref bytes } = &__binrw_this;
                        (|| { Ok(()) })().map(|_: ()| __binrw_this)
                    })
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    impl binrw::meta::ReadEndian for StfsEntryFlags {
        const ENDIAN: binrw::meta::EndianKind = binrw::meta::EndianKind::None;
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for StfsEntryFlags {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            binrw::BinWrite::write_options(
                &((|flags: &Self| flags.into_bytes())(self)),
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                (),
            )?;
            Ok(())
        }
    }
    impl binrw::meta::WriteEndian for StfsEntryFlags {
        const ENDIAN: binrw::meta::EndianKind = binrw::meta::EndianKind::None;
    }
    #[allow(clippy::identity_op)]
    const _: () = {
        impl ::modular_bitfield::private::checks::CheckTotalSizeMultipleOf8
        for StfsEntryFlags {
            type Size = ::modular_bitfield::private::checks::TotalSize<
                [(); {
                    0usize + <B6 as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS
                } % 8usize],
            >;
        }
    };
    impl StfsEntryFlags {
        /// Returns an instance with zero initialized data.
        #[allow(clippy::identity_op)]
        pub const fn new() -> Self {
            Self {
                bytes: [0u8; {
                    ((({
                        0usize + <B6 as ::modular_bitfield::Specifier>::BITS
                            + <bool as ::modular_bitfield::Specifier>::BITS
                            + <bool as ::modular_bitfield::Specifier>::BITS
                    } - 1) / 8) + 1) * 8
                } / 8usize],
            }
        }
    }
    impl StfsEntryFlags {
        /// Returns the underlying bits.
        ///
        /// # Layout
        ///
        /// The returned byte array is layed out in the same way as described
        /// [here](https://docs.rs/modular-bitfield/#generated-structure).
        #[inline]
        #[allow(clippy::identity_op)]
        pub const fn into_bytes(
            self,
        ) -> [::core::primitive::u8; {
            ((({
                0usize + <B6 as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS
            } - 1) / 8) + 1) * 8
        } / 8usize] {
            self.bytes
        }
        /// Converts the given bytes directly into the bitfield struct.
        #[inline]
        #[allow(clippy::identity_op)]
        pub const fn from_bytes(
            bytes: [::core::primitive::u8; {
                ((({
                    0usize + <B6 as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS
                } - 1) / 8) + 1) * 8
            } / 8usize],
        ) -> Self {
            Self { bytes }
        }
    }
    const _: () = {
        const _: () = {};
        const _: () = {};
        const _: () = {};
    };
    impl StfsEntryFlags {
        ///Returns the value of name_len.
        #[inline]
        fn name_len(&self) -> <B6 as ::modular_bitfield::Specifier>::InOut {
            self.name_len_or_err()
                .expect(
                    "value contains invalid bit pattern for field StfsEntryFlags.name_len",
                )
        }
        /**Returns the value of name_len.

#Errors

If the returned value contains an invalid bit pattern for name_len.*/
        #[inline]
        #[allow(dead_code)]
        fn name_len_or_err(
            &self,
        ) -> ::core::result::Result<
            <B6 as ::modular_bitfield::Specifier>::InOut,
            ::modular_bitfield::error::InvalidBitPattern<
                <B6 as ::modular_bitfield::Specifier>::Bytes,
            >,
        > {
            let __bf_read: <B6 as ::modular_bitfield::Specifier>::Bytes = {
                ::modular_bitfield::private::read_specifier::<
                    B6,
                >(&self.bytes[..], 0usize)
            };
            <B6 as ::modular_bitfield::Specifier>::from_bytes(__bf_read)
        }
        /**Returns a copy of the bitfield with the value of name_len set to the given value.

#Panics

If the given value is out of bounds for name_len.*/
        #[inline]
        #[allow(dead_code)]
        fn with_name_len(
            mut self,
            new_val: <B6 as ::modular_bitfield::Specifier>::InOut,
        ) -> Self {
            self.set_name_len(new_val);
            self
        }
        /**Returns a copy of the bitfield with the value of name_len set to the given value.

#Errors

If the given value is out of bounds for name_len.*/
        #[inline]
        #[allow(dead_code)]
        fn with_name_len_checked(
            mut self,
            new_val: <B6 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<Self, ::modular_bitfield::error::OutOfBounds> {
            self.set_name_len_checked(new_val)?;
            ::core::result::Result::Ok(self)
        }
        /**Sets the value of name_len to the given value.

#Panics

If the given value is out of bounds for name_len.*/
        #[inline]
        #[allow(dead_code)]
        fn set_name_len(
            &mut self,
            new_val: <B6 as ::modular_bitfield::Specifier>::InOut,
        ) {
            self.set_name_len_checked(new_val)
                .expect("value out of bounds for field StfsEntryFlags.name_len")
        }
        /**Sets the value of name_len to the given value.

#Errors

If the given value is out of bounds for name_len.*/
        #[inline]
        fn set_name_len_checked(
            &mut self,
            new_val: <B6 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<(), ::modular_bitfield::error::OutOfBounds> {
            let __bf_base_bits: ::core::primitive::usize = 8usize
                * ::core::mem::size_of::<<B6 as ::modular_bitfield::Specifier>::Bytes>();
            let __bf_max_value: <B6 as ::modular_bitfield::Specifier>::Bytes = {
                !0 >> (__bf_base_bits - <B6 as ::modular_bitfield::Specifier>::BITS)
            };
            let __bf_spec_bits: ::core::primitive::usize = <B6 as ::modular_bitfield::Specifier>::BITS;
            let __bf_raw_val: <B6 as ::modular_bitfield::Specifier>::Bytes = {
                <B6 as ::modular_bitfield::Specifier>::into_bytes(new_val)
            }?;
            if !(__bf_base_bits == __bf_spec_bits || __bf_raw_val <= __bf_max_value) {
                return ::core::result::Result::Err(
                    ::modular_bitfield::error::OutOfBounds,
                );
            }
            ::modular_bitfield::private::write_specifier::<
                B6,
            >(&mut self.bytes[..], 0usize, __bf_raw_val);
            ::core::result::Result::Ok(())
        }
        ///Returns the value of has_consecutive_blocks.
        #[inline]
        fn has_consecutive_blocks(
            &self,
        ) -> <bool as ::modular_bitfield::Specifier>::InOut {
            self.has_consecutive_blocks_or_err()
                .expect(
                    "value contains invalid bit pattern for field StfsEntryFlags.has_consecutive_blocks",
                )
        }
        /**Returns the value of has_consecutive_blocks.

#Errors

If the returned value contains an invalid bit pattern for has_consecutive_blocks.*/
        #[inline]
        #[allow(dead_code)]
        fn has_consecutive_blocks_or_err(
            &self,
        ) -> ::core::result::Result<
            <bool as ::modular_bitfield::Specifier>::InOut,
            ::modular_bitfield::error::InvalidBitPattern<
                <bool as ::modular_bitfield::Specifier>::Bytes,
            >,
        > {
            let __bf_read: <bool as ::modular_bitfield::Specifier>::Bytes = {
                ::modular_bitfield::private::read_specifier::<
                    bool,
                >(&self.bytes[..], 0usize + <B6 as ::modular_bitfield::Specifier>::BITS)
            };
            <bool as ::modular_bitfield::Specifier>::from_bytes(__bf_read)
        }
        /**Returns a copy of the bitfield with the value of has_consecutive_blocks set to the given value.

#Panics

If the given value is out of bounds for has_consecutive_blocks.*/
        #[inline]
        #[allow(dead_code)]
        fn with_has_consecutive_blocks(
            mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> Self {
            self.set_has_consecutive_blocks(new_val);
            self
        }
        /**Returns a copy of the bitfield with the value of has_consecutive_blocks set to the given value.

#Errors

If the given value is out of bounds for has_consecutive_blocks.*/
        #[inline]
        #[allow(dead_code)]
        fn with_has_consecutive_blocks_checked(
            mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<Self, ::modular_bitfield::error::OutOfBounds> {
            self.set_has_consecutive_blocks_checked(new_val)?;
            ::core::result::Result::Ok(self)
        }
        /**Sets the value of has_consecutive_blocks to the given value.

#Panics

If the given value is out of bounds for has_consecutive_blocks.*/
        #[inline]
        #[allow(dead_code)]
        fn set_has_consecutive_blocks(
            &mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) {
            self.set_has_consecutive_blocks_checked(new_val)
                .expect(
                    "value out of bounds for field StfsEntryFlags.has_consecutive_blocks",
                )
        }
        /**Sets the value of has_consecutive_blocks to the given value.

#Errors

If the given value is out of bounds for has_consecutive_blocks.*/
        #[inline]
        fn set_has_consecutive_blocks_checked(
            &mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<(), ::modular_bitfield::error::OutOfBounds> {
            let __bf_base_bits: ::core::primitive::usize = 8usize
                * ::core::mem::size_of::<
                    <bool as ::modular_bitfield::Specifier>::Bytes,
                >();
            let __bf_max_value: <bool as ::modular_bitfield::Specifier>::Bytes = {
                !0 >> (__bf_base_bits - <bool as ::modular_bitfield::Specifier>::BITS)
            };
            let __bf_spec_bits: ::core::primitive::usize = <bool as ::modular_bitfield::Specifier>::BITS;
            let __bf_raw_val: <bool as ::modular_bitfield::Specifier>::Bytes = {
                <bool as ::modular_bitfield::Specifier>::into_bytes(new_val)
            }?;
            if !(__bf_base_bits == __bf_spec_bits || __bf_raw_val <= __bf_max_value) {
                return ::core::result::Result::Err(
                    ::modular_bitfield::error::OutOfBounds,
                );
            }
            ::modular_bitfield::private::write_specifier::<
                bool,
            >(
                &mut self.bytes[..],
                0usize + <B6 as ::modular_bitfield::Specifier>::BITS,
                __bf_raw_val,
            );
            ::core::result::Result::Ok(())
        }
        ///Returns the value of is_folder.
        #[inline]
        fn is_folder(&self) -> <bool as ::modular_bitfield::Specifier>::InOut {
            self.is_folder_or_err()
                .expect(
                    "value contains invalid bit pattern for field StfsEntryFlags.is_folder",
                )
        }
        /**Returns the value of is_folder.

#Errors

If the returned value contains an invalid bit pattern for is_folder.*/
        #[inline]
        #[allow(dead_code)]
        fn is_folder_or_err(
            &self,
        ) -> ::core::result::Result<
            <bool as ::modular_bitfield::Specifier>::InOut,
            ::modular_bitfield::error::InvalidBitPattern<
                <bool as ::modular_bitfield::Specifier>::Bytes,
            >,
        > {
            let __bf_read: <bool as ::modular_bitfield::Specifier>::Bytes = {
                ::modular_bitfield::private::read_specifier::<
                    bool,
                >(
                    &self.bytes[..],
                    0usize + <B6 as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS,
                )
            };
            <bool as ::modular_bitfield::Specifier>::from_bytes(__bf_read)
        }
        /**Returns a copy of the bitfield with the value of is_folder set to the given value.

#Panics

If the given value is out of bounds for is_folder.*/
        #[inline]
        #[allow(dead_code)]
        fn with_is_folder(
            mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> Self {
            self.set_is_folder(new_val);
            self
        }
        /**Returns a copy of the bitfield with the value of is_folder set to the given value.

#Errors

If the given value is out of bounds for is_folder.*/
        #[inline]
        #[allow(dead_code)]
        fn with_is_folder_checked(
            mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<Self, ::modular_bitfield::error::OutOfBounds> {
            self.set_is_folder_checked(new_val)?;
            ::core::result::Result::Ok(self)
        }
        /**Sets the value of is_folder to the given value.

#Panics

If the given value is out of bounds for is_folder.*/
        #[inline]
        #[allow(dead_code)]
        fn set_is_folder(
            &mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) {
            self.set_is_folder_checked(new_val)
                .expect("value out of bounds for field StfsEntryFlags.is_folder")
        }
        /**Sets the value of is_folder to the given value.

#Errors

If the given value is out of bounds for is_folder.*/
        #[inline]
        fn set_is_folder_checked(
            &mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<(), ::modular_bitfield::error::OutOfBounds> {
            let __bf_base_bits: ::core::primitive::usize = 8usize
                * ::core::mem::size_of::<
                    <bool as ::modular_bitfield::Specifier>::Bytes,
                >();
            let __bf_max_value: <bool as ::modular_bitfield::Specifier>::Bytes = {
                !0 >> (__bf_base_bits - <bool as ::modular_bitfield::Specifier>::BITS)
            };
            let __bf_spec_bits: ::core::primitive::usize = <bool as ::modular_bitfield::Specifier>::BITS;
            let __bf_raw_val: <bool as ::modular_bitfield::Specifier>::Bytes = {
                <bool as ::modular_bitfield::Specifier>::into_bytes(new_val)
            }?;
            if !(__bf_base_bits == __bf_spec_bits || __bf_raw_val <= __bf_max_value) {
                return ::core::result::Result::Err(
                    ::modular_bitfield::error::OutOfBounds,
                );
            }
            ::modular_bitfield::private::write_specifier::<
                bool,
            >(
                &mut self.bytes[..],
                0usize + <B6 as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS,
                __bf_raw_val,
            );
            ::core::result::Result::Ok(())
        }
    }
    impl ::core::fmt::Debug for StfsEntryFlags {
        fn fmt(&self, __bf_f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
            __bf_f
                .debug_struct("StfsEntryFlags")
                .field(
                    "name_len",
                    self
                        .name_len_or_err()
                        .as_ref()
                        .map(|__bf_field| __bf_field as &dyn ::core::fmt::Debug)
                        .unwrap_or_else(|__bf_err| __bf_err as &dyn ::core::fmt::Debug),
                )
                .field(
                    "has_consecutive_blocks",
                    self
                        .has_consecutive_blocks_or_err()
                        .as_ref()
                        .map(|__bf_field| __bf_field as &dyn ::core::fmt::Debug)
                        .unwrap_or_else(|__bf_err| __bf_err as &dyn ::core::fmt::Debug),
                )
                .field(
                    "is_folder",
                    self
                        .is_folder_or_err()
                        .as_ref()
                        .map(|__bf_field| __bf_field as &dyn ::core::fmt::Debug)
                        .unwrap_or_else(|__bf_err| __bf_err as &dyn ::core::fmt::Debug),
                )
                .finish()
        }
    }
    pub struct HashTable {
        level: HashTableLevel,
        true_block_number: Block,
        entry_count: usize,
        address_in_file: usize,
        entries: Vec<HashEntry>,
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for HashTable {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field5_finish(
                f,
                "HashTable",
                "level",
                &self.level,
                "true_block_number",
                &self.true_block_number,
                "entry_count",
                &self.entry_count,
                "address_in_file",
                &self.address_in_file,
                "entries",
                &&self.entries,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for HashTable {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "HashTable",
                    false as usize + 1 + 1 + 1 + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "level",
                    &self.level,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "true_block_number",
                    &self.true_block_number,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "entry_count",
                    &self.entry_count,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "address_in_file",
                    &self.address_in_file,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "entries",
                    &self.entries,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    impl Default for HashTable {
        fn default() -> Self {
            HashTable {
                level: HashTableLevel::First,
                true_block_number: Block(0),
                entry_count: 0,
                address_in_file: 0,
                entries: Vec::default(),
            }
        }
    }
    impl HashTable {
        /// Reads top-level hashes
        pub fn parse_hash_entries(&mut self, data: &[u8]) -> Result<(), StfsError> {
            let mut reader = Cursor::new(data);
            for _ in 0..self.entry_count {
                let entry = reader.read_be::<HashEntry>()?;
                self.entries.push(entry);
            }
            Ok(())
        }
        /// Returns the file range (start..end offset) this hash table occupies
        pub fn data_range(&self) -> Range<usize> {
            self
                .address_in_file..(self.address_in_file
                + self.entry_count * (std::mem::size_of::<HashEntry>() - 1))
        }
    }
    pub enum HashTableLevel {
        First,
        Second,
        Third,
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for HashTableLevel {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    HashTableLevel::First => "First",
                    HashTableLevel::Second => "Second",
                    HashTableLevel::Third => "Third",
                },
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for HashTableLevel {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    HashTableLevel::First => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "HashTableLevel",
                            0u32,
                            "First",
                        )
                    }
                    HashTableLevel::Second => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "HashTableLevel",
                            1u32,
                            "Second",
                        )
                    }
                    HashTableLevel::Third => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "HashTableLevel",
                            2u32,
                            "Third",
                        )
                    }
                }
            }
        }
    };
    #[automatically_derived]
    impl ::core::marker::Copy for HashTableLevel {}
    #[automatically_derived]
    impl ::core::clone::Clone for HashTableLevel {
        #[inline]
        fn clone(&self) -> HashTableLevel {
            *self
        }
    }
    enum KeyMaterial {
        /// Only present in console-signed packages
        Certificate(Certificate),
        /// Only present in strong-signed packages
        Signature(Vec<u8>),
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for KeyMaterial {
        type Args<'__binrw_generated_args_lifetime> = (PackageType,);
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let (mut package_type,) = __binrw_generated_var_arguments;
                let __binrw_generated_var_endian = binrw::Endian::Big;
                extern crate alloc;
                let mut __binrw_generated_error_basket: alloc::vec::Vec<
                    (&'static str, binrw::Error),
                > = alloc::vec::Vec::new();
                match (|| {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    binrw::__private::assert(
                        package_type == PackageType::Con,
                        __binrw_generated_position_temp,
                        binrw::__private::AssertErrorFn::<
                            _,
                            fn() -> !,
                        >::Message(|| {
                            extern crate alloc;
                            {
                                let res = ::alloc::fmt::format(
                                    format_args!(
                                        "assertion failed: `{0}`",
                                        "package_type == PackageType :: Con",
                                    ),
                                );
                                res
                            }
                        }),
                    )?;
                    let __binrw_generated_read_function = binrw::BinRead::read_options;
                    let mut self_0: Certificate = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'self_0' in KeyMaterial::Certificate"
                                    .into(),
                                line: 646u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   \u{1b}[1m652\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mCertificate\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    let __binrw_this = Self::Certificate(self_0);
                    Ok(__binrw_this)
                })() {
                    ok @ Ok(_) => return ok,
                    Err(error) => {
                        binrw::__private::restore_position_variant(
                                __binrw_generated_var_reader,
                                __binrw_generated_position_temp,
                                error,
                            )
                            .map(|__binrw_temp| {
                                __binrw_generated_error_basket
                                    .push(("Certificate", __binrw_temp));
                            })?;
                    }
                }
                match (|| {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    binrw::__private::assert(
                        package_type != PackageType::Con,
                        __binrw_generated_position_temp,
                        binrw::__private::AssertErrorFn::<
                            _,
                            fn() -> !,
                        >::Message(|| {
                            extern crate alloc;
                            {
                                let res = ::alloc::fmt::format(
                                    format_args!(
                                        "assertion failed: `{0}`",
                                        "package_type != PackageType :: Con",
                                    ),
                                );
                                res
                            }
                        }),
                    )?;
                    let __binrw_generated_read_function = binrw::BinRead::read_options;
                    let __binrw_generated_args_self_0: <Vec<
                        u8,
                    > as binrw::BinRead>::Args<'_> = {
                        let args_ty = ::core::marker::PhantomData::<_>;
                        if false {
                            ::binrw::__private::passthrough_helper(args_ty)
                        } else {
                            let builder = ::binrw::__private::builder_helper(args_ty);
                            let builder = builder
                                .count({
                                    let __binrw_temp = 64;
                                    #[allow(clippy::useless_conversion)]
                                    usize::try_from(__binrw_temp)
                                        .map_err(|_| {
                                            extern crate alloc;
                                            binrw::Error::AssertFail {
                                                pos: binrw::io::Seek::stream_position(
                                                        __binrw_generated_var_reader,
                                                    )
                                                    .unwrap_or_default(),
                                                message: {
                                                    let res = ::alloc::fmt::format(
                                                        format_args!(
                                                            "count {0:?} out of range of usize",
                                                            __binrw_temp,
                                                        ),
                                                    );
                                                    res
                                                },
                                            }
                                        })?
                                });
                            builder.finalize()
                        }
                    };
                    let mut self_0: Vec<u8> = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            __binrw_generated_args_self_0,
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'self_0' in KeyMaterial::Signature"
                                    .into(),
                                line: 646u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   \u{1b}[1m656\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1m\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197m#\u{1b}[39m\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mcount\u{1b}[39m\u{1b}[0m\u{1b}[1m = \u{1b}[0m\u{1b}[1m\u{1b}[38;5;135m64\u{1b}[39m\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197m]\u{1b}[39m\u{1b}[0m\u{1b}[1m \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mVec\u{1b}[39m\u{1b}[0m\u{1b}[1m<\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m>\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    let __binrw_this = Self::Signature(self_0);
                    Ok(__binrw_this)
                })() {
                    ok @ Ok(_) => return ok,
                    Err(error) => {
                        binrw::__private::restore_position_variant(
                                __binrw_generated_var_reader,
                                __binrw_generated_position_temp,
                                error,
                            )
                            .map(|__binrw_temp| {
                                __binrw_generated_error_basket
                                    .push(("Signature", __binrw_temp));
                            })?;
                    }
                }
                Err(binrw::Error::EnumErrors {
                    pos: __binrw_generated_position_temp,
                    variant_errors: __binrw_generated_error_basket,
                })
            })()
        }
    }
    impl binrw::meta::ReadEndian for KeyMaterial {
        const ENDIAN: binrw::meta::EndianKind = binrw::meta::EndianKind::Endian(
            binrw::Endian::Big,
        );
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for KeyMaterial {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = binrw::Endian::Big;
            match self {
                Self::Certificate(ref self_0) => {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                        binrw::BinWrite::write_options,
                    );
                    let __binrw_generated_args_self_0: <Certificate as binrw::BinWrite>::Args<
                        '_,
                    > = <_ as binrw::__private::Required>::args();
                    __binrw_generated_write_function(
                        &self_0,
                        __binrw_generated_var_writer,
                        __binrw_generated_var_endian,
                        __binrw_generated_args_self_0,
                    )?;
                }
                Self::Signature(ref self_0) => {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                        binrw::BinWrite::write_options,
                    );
                    let __binrw_generated_args_self_0: <Vec<
                        u8,
                    > as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
                    __binrw_generated_write_function(
                        &self_0,
                        __binrw_generated_var_writer,
                        __binrw_generated_var_endian,
                        __binrw_generated_args_self_0,
                    )?;
                }
            }
            Ok(())
        }
    }
    impl binrw::meta::WriteEndian for KeyMaterial {
        const ENDIAN: binrw::meta::EndianKind = binrw::meta::EndianKind::Endian(
            binrw::Endian::Big,
        );
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for KeyMaterial {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            match self {
                KeyMaterial::Certificate(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "Certificate",
                        &__self_0,
                    )
                }
                KeyMaterial::Signature(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "Signature",
                        &__self_0,
                    )
                }
            }
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for KeyMaterial {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    KeyMaterial::Certificate(ref __field0) => {
                        _serde::Serializer::serialize_newtype_variant(
                            __serializer,
                            "KeyMaterial",
                            0u32,
                            "Certificate",
                            __field0,
                        )
                    }
                    KeyMaterial::Signature(ref __field0) => {
                        _serde::Serializer::serialize_newtype_variant(
                            __serializer,
                            "KeyMaterial",
                            1u32,
                            "Signature",
                            __field0,
                        )
                    }
                }
            }
        }
    };
    pub enum XContentHeaderMetadata {
        XContentPackage(XContentHeader),
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for XContentHeaderMetadata {
        type Args<'__binrw_generated_args_lifetime> = (bool,);
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let (mut is_profile_embedded_content,) = __binrw_generated_var_arguments;
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                extern crate alloc;
                let mut __binrw_generated_error_basket: alloc::vec::Vec<
                    (&'static str, binrw::Error),
                > = alloc::vec::Vec::new();
                match (|| {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    binrw::__private::assert(
                        !is_profile_embedded_content,
                        __binrw_generated_position_temp,
                        binrw::__private::AssertErrorFn::<
                            _,
                            fn() -> !,
                        >::Message(|| {
                            extern crate alloc;
                            {
                                let res = ::alloc::fmt::format(
                                    format_args!(
                                        "assertion failed: `{0}`",
                                        "! is_profile_embedded_content",
                                    ),
                                );
                                res
                            }
                        }),
                    )?;
                    let __binrw_generated_read_function = binrw::BinRead::read_options;
                    let mut self_0: XContentHeader = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'self_0' in XContentHeaderMetadata::XContentPackage"
                                    .into(),
                                line: 660u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   \u{1b}[1m664\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mXContentHeader\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    let __binrw_this = Self::XContentPackage(self_0);
                    Ok(__binrw_this)
                })() {
                    ok @ Ok(_) => return ok,
                    Err(error) => {
                        binrw::__private::restore_position_variant(
                                __binrw_generated_var_reader,
                                __binrw_generated_position_temp,
                                error,
                            )
                            .map(|__binrw_temp| {
                                __binrw_generated_error_basket
                                    .push(("XContentPackage", __binrw_temp));
                            })?;
                    }
                }
                Err(binrw::Error::EnumErrors {
                    pos: __binrw_generated_position_temp,
                    variant_errors: __binrw_generated_error_basket,
                })
            })()
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for XContentHeaderMetadata {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            match self {
                Self::XContentPackage(ref self_0) => {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                        binrw::BinWrite::write_options,
                    );
                    let __binrw_generated_args_self_0: <XContentHeader as binrw::BinWrite>::Args<
                        '_,
                    > = <_ as binrw::__private::Required>::args();
                    __binrw_generated_write_function(
                        &self_0,
                        __binrw_generated_var_writer,
                        __binrw_generated_var_endian,
                        __binrw_generated_args_self_0,
                    )?;
                }
            }
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for XContentHeaderMetadata {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            match self {
                XContentHeaderMetadata::XContentPackage(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "XContentPackage",
                        &__self_0,
                    )
                }
            }
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for XContentHeaderMetadata {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    XContentHeaderMetadata::XContentPackage(ref __field0) => {
                        _serde::Serializer::serialize_newtype_variant(
                            __serializer,
                            "XContentHeaderMetadata",
                            0u32,
                            "XContentPackage",
                            __field0,
                        )
                    }
                }
            }
        }
    };
    pub struct XContentHeader {
        pub package_type: PackageType,
        pub key_material: KeyMaterial,
        pub license_data: [LicenseEntry; 0x10],
        pub header_hash: [u8; 0x14],
        pub header_size: u32,
        pub content_type: ContentType,
        pub metadata_version: u32,
        pub content_size: u64,
        pub media_id: u32,
        pub version: u32,
        pub base_version: u32,
        pub title_id: u32,
        pub platform: u8,
        pub executable_type: u8,
        pub disc_number: u8,
        pub disc_in_set: u8,
        pub savegame_id: u32,
        pub console_id: [u8; 5],
        pub profile_id: u64,
        pub volume_kind: FileSystemKind,
        pub volume_descriptor: FileSystem,
        pub data_file_count: u32,
        pub data_file_combined_size: u64,
        pub device_id: [u8; 0x14],
        #[serde(serialize_with = "serialize_null_wide_string")]
        pub display_name: NullWideString,
        #[serde(serialize_with = "serialize_null_wide_string")]
        pub display_description: NullWideString,
        #[serde(serialize_with = "serialize_null_wide_string")]
        pub publisher_name: NullWideString,
        #[serde(serialize_with = "serialize_null_wide_string")]
        pub title_name: NullWideString,
        pub transfer_flags: u8,
        pub thumbnail_image_size: u32,
        pub title_thumbnail_image_size: u32,
        pub thumbnail_image: Vec<u8>,
        pub title_image: Vec<u8>,
        pub installer_type: Option<InstallerType>,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for XContentHeader {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = binrw::Endian::Big;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut package_type: PackageType = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'package_type' in XContentHeader"
                                .into(),
                            line: 671u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m671\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub package_type: PackageType\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let __binrw_generated_args_key_material: <KeyMaterial as binrw::BinRead>::Args<
                    '_,
                > = (package_type,);
                let mut key_material: KeyMaterial = {
                    let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                        __binrw_generated_var_reader,
                    )?;
                    let __binrw_temp = {
                        let __binrw_generated_saved_position = binrw::io::Seek::stream_position(
                            __binrw_generated_var_reader,
                        )?;
                        let __binrw_temp = __binrw_generated_read_function(
                                __binrw_generated_var_reader,
                                __binrw_generated_var_endian,
                                __binrw_generated_args_key_material,
                            )
                            .map_err(|err| binrw::error::ContextExt::with_context(
                                err,
                                binrw::error::BacktraceFrame::Full {
                                    message: "While parsing field 'key_material' in XContentHeader"
                                        .into(),
                                    line: 673u32,
                                    file: "stfs/src/parse.rs",
                                    code: Some(
                                        "  ┄────╮\n   672 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197margs\u{1b}[39m(package_type), \u{1b}[38;5;197mdbg\u{1b}[39m, \u{1b}[38;5;197mpad_size_to\u{1b}[39m = \u{1b}[38;5;135m0x228\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m673\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub key_material: KeyMaterial\u{1b}[0m\n  ┄────╯\n",
                                    ),
                                },
                            ))?;
                        {
                            ::std::io::_eprint(
                                format_args!(
                                    "[{0}:{1} | offset {2:#x}] {3} = {4:#x?}\n",
                                    "stfs/src/parse.rs",
                                    673usize,
                                    __binrw_generated_saved_position,
                                    "key_material",
                                    &__binrw_temp,
                                ),
                            );
                        };
                        {
                            {
                                ::std::io::_eprint(
                                    format_args!(
                                        "[{0}:{1} | pad_size_to {2:#x}]\n",
                                        "stfs/src/parse.rs",
                                        673usize,
                                        0x228,
                                    ),
                                );
                            };
                        }
                        __binrw_temp
                    };
                    {
                        let pad = (0x228) as i64;
                        let size = (binrw::io::Seek::stream_position(
                            __binrw_generated_var_reader,
                        )? - __binrw_generated_position_temp) as i64;
                        if size < pad {
                            binrw::io::Seek::seek(
                                __binrw_generated_var_reader,
                                binrw::io::SeekFrom::Current(pad - size),
                            )?;
                        }
                    }
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut license_data: [LicenseEntry; 0x10] = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'license_data' in XContentHeader"
                                .into(),
                            line: 675u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m675\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub license_data: [LicenseEntry; \u{1b}[0m\u{1b}[1m\u{1b}[38;5;135m0x10\u{1b}[39m\u{1b}[0m\u{1b}[1m]\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut header_hash: [u8; 0x14] = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'header_hash' in XContentHeader"
                                .into(),
                            line: 676u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m676\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub header_hash: [\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m; \u{1b}[0m\u{1b}[1m\u{1b}[38;5;135m0x14\u{1b}[39m\u{1b}[0m\u{1b}[1m]\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut header_size: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'header_size' in XContentHeader"
                                .into(),
                            line: 677u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m677\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub header_size: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut content_type: ContentType = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'content_type' in XContentHeader"
                                .into(),
                            line: 679u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m679\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub content_type: ContentType\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut metadata_version: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'metadata_version' in XContentHeader"
                                .into(),
                            line: 680u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m680\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub metadata_version: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut content_size: u64 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'content_size' in XContentHeader"
                                .into(),
                            line: 681u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m681\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub content_size: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu64\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut media_id: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'media_id' in XContentHeader"
                                .into(),
                            line: 682u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m682\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub media_id: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut version: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'version' in XContentHeader"
                                .into(),
                            line: 683u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m683\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub version: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut base_version: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'base_version' in XContentHeader"
                                .into(),
                            line: 684u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m684\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub base_version: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut title_id: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'title_id' in XContentHeader"
                                .into(),
                            line: 685u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m685\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub title_id: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut platform: u8 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'platform' in XContentHeader"
                                .into(),
                            line: 686u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m686\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub platform: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut executable_type: u8 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'executable_type' in XContentHeader"
                                .into(),
                            line: 687u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m687\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub executable_type: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut disc_number: u8 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'disc_number' in XContentHeader"
                                .into(),
                            line: 688u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m688\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub disc_number: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut disc_in_set: u8 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'disc_in_set' in XContentHeader"
                                .into(),
                            line: 689u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m689\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub disc_in_set: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut savegame_id: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'savegame_id' in XContentHeader"
                                .into(),
                            line: 690u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m690\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub savegame_id: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut console_id: [u8; 5] = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'console_id' in XContentHeader"
                                .into(),
                            line: 691u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m691\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub console_id: [\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m; \u{1b}[0m\u{1b}[1m\u{1b}[38;5;135m5\u{1b}[39m\u{1b}[0m\u{1b}[1m]\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut profile_id: u64 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'profile_id' in XContentHeader"
                                .into(),
                            line: 692u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m692\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub profile_id: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu64\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut volume_kind: FileSystemKind = {
                    binrw::io::Seek::seek(
                        __binrw_generated_var_reader,
                        std::io::SeekFrom::Start(0x3a9),
                    )?;
                    let __binrw_temp = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'volume_kind' in XContentHeader"
                                    .into(),
                                line: 695u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   694 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mseek_before\u{1b}[39m = \u{1b}[38;5;197mstd\u{1b}[39m::io::SeekFrom::\u{1b}[38;5;148mStart\u{1b}[39m(\u{1b}[38;5;135m0x3a9\u{1b}[39m)\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m695\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub volume_kind: FileSystemKind\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let __binrw_generated_args_volume_descriptor: <FileSystem as binrw::BinRead>::Args<
                    '_,
                > = (volume_kind,);
                let mut volume_descriptor: FileSystem = {
                    binrw::io::Seek::seek(
                        __binrw_generated_var_reader,
                        std::io::SeekFrom::Start(0x379),
                    )?;
                    let __binrw_temp = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            __binrw_generated_args_volume_descriptor,
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'volume_descriptor' in XContentHeader"
                                    .into(),
                                line: 699u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   697 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mseek_before\u{1b}[39m = \u{1b}[38;5;197mstd\u{1b}[39m::io::SeekFrom::\u{1b}[38;5;148mStart\u{1b}[39m(\u{1b}[38;5;135m0x379\u{1b}[39m)\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   698 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197margs\u{1b}[39m(volume_kind)\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m699\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub volume_descriptor: FileSystem\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut data_file_count: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'data_file_count' in XContentHeader"
                                .into(),
                            line: 702u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m702\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub data_file_count: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut data_file_combined_size: u64 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'data_file_combined_size' in XContentHeader"
                                .into(),
                            line: 703u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m703\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub data_file_combined_size: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu64\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut device_id: [u8; 0x14] = {
                    binrw::io::Seek::seek(
                        __binrw_generated_var_reader,
                        std::io::SeekFrom::Start(0x3fd),
                    )?;
                    let __binrw_temp = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'device_id' in XContentHeader"
                                    .into(),
                                line: 707u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   706 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mseek_before\u{1b}[39m = \u{1b}[38;5;197mstd\u{1b}[39m::io::SeekFrom::\u{1b}[38;5;148mStart\u{1b}[39m(\u{1b}[38;5;135m0x3fd\u{1b}[39m)\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m707\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub device_id: [\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m; \u{1b}[0m\u{1b}[1m\u{1b}[38;5;135m0x14\u{1b}[39m\u{1b}[0m\u{1b}[1m]\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut display_name: NullWideString = {
                    let __binrw_generated_saved_position = binrw::io::Seek::stream_position(
                        __binrw_generated_var_reader,
                    )?;
                    let __binrw_temp = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'display_name' in XContentHeader"
                                    .into(),
                                line: 712u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   710 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mserde\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39mserialize_with = \"serialize_null_wide_string\"\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   711 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mdbg\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m712\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub display_name: NullWideString\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    {
                        ::std::io::_eprint(
                            format_args!(
                                "[{0}:{1} | offset {2:#x}] {3} = {4:#x?}\n",
                                "stfs/src/parse.rs",
                                712usize,
                                __binrw_generated_saved_position,
                                "display_name",
                                &__binrw_temp,
                            ),
                        );
                    };
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut display_description: NullWideString = {
                    binrw::io::Seek::seek(
                        __binrw_generated_var_reader,
                        std::io::SeekFrom::Start(0xd11),
                    )?;
                    let __binrw_temp = {
                        let __binrw_generated_saved_position = binrw::io::Seek::stream_position(
                            __binrw_generated_var_reader,
                        )?;
                        let __binrw_temp = __binrw_generated_read_function(
                                __binrw_generated_var_reader,
                                __binrw_generated_var_endian,
                                <_ as binrw::__private::Required>::args(),
                            )
                            .map_err(|err| binrw::error::ContextExt::with_context(
                                err,
                                binrw::error::BacktraceFrame::Full {
                                    message: "While parsing field 'display_description' in XContentHeader"
                                        .into(),
                                    line: 717u32,
                                    file: "stfs/src/parse.rs",
                                    code: Some(
                                        "  ┄────╮\n   714 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mserde\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39mserialize_with = \"serialize_null_wide_string\"\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   715 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mseek_before\u{1b}[39m = \u{1b}[38;5;197mstd\u{1b}[39m::io::SeekFrom::\u{1b}[38;5;148mStart\u{1b}[39m(\u{1b}[38;5;135m0xd11\u{1b}[39m)\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   716 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mdbg\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m717\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub display_description: NullWideString\u{1b}[0m\n  ┄────╯\n",
                                    ),
                                },
                            ))?;
                        {
                            ::std::io::_eprint(
                                format_args!(
                                    "[{0}:{1} | offset {2:#x}] {3} = {4:#x?}\n",
                                    "stfs/src/parse.rs",
                                    717usize,
                                    __binrw_generated_saved_position,
                                    "display_description",
                                    &__binrw_temp,
                                ),
                            );
                        };
                        __binrw_temp
                    };
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut publisher_name: NullWideString = {
                    binrw::io::Seek::seek(
                        __binrw_generated_var_reader,
                        std::io::SeekFrom::Start(0x1611),
                    )?;
                    let __binrw_temp = {
                        let __binrw_generated_saved_position = binrw::io::Seek::stream_position(
                            __binrw_generated_var_reader,
                        )?;
                        let __binrw_temp = __binrw_generated_read_function(
                                __binrw_generated_var_reader,
                                __binrw_generated_var_endian,
                                <_ as binrw::__private::Required>::args(),
                            )
                            .map_err(|err| binrw::error::ContextExt::with_context(
                                err,
                                binrw::error::BacktraceFrame::Full {
                                    message: "While parsing field 'publisher_name' in XContentHeader"
                                        .into(),
                                    line: 722u32,
                                    file: "stfs/src/parse.rs",
                                    code: Some(
                                        "  ┄────╮\n   719 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mserde\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39mserialize_with = \"serialize_null_wide_string\"\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   720 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mseek_before\u{1b}[39m = \u{1b}[38;5;197mstd\u{1b}[39m::io::SeekFrom::\u{1b}[38;5;148mStart\u{1b}[39m(\u{1b}[38;5;135m0x1611\u{1b}[39m)\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   721 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mdbg\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m722\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub publisher_name: NullWideString\u{1b}[0m\n  ┄────╯\n",
                                    ),
                                },
                            ))?;
                        {
                            ::std::io::_eprint(
                                format_args!(
                                    "[{0}:{1} | offset {2:#x}] {3} = {4:#x?}\n",
                                    "stfs/src/parse.rs",
                                    722usize,
                                    __binrw_generated_saved_position,
                                    "publisher_name",
                                    &__binrw_temp,
                                ),
                            );
                        };
                        __binrw_temp
                    };
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut title_name: NullWideString = {
                    binrw::io::Seek::seek(
                        __binrw_generated_var_reader,
                        std::io::SeekFrom::Start(0x1691),
                    )?;
                    let __binrw_temp = {
                        let __binrw_generated_saved_position = binrw::io::Seek::stream_position(
                            __binrw_generated_var_reader,
                        )?;
                        let __binrw_temp = __binrw_generated_read_function(
                                __binrw_generated_var_reader,
                                __binrw_generated_var_endian,
                                <_ as binrw::__private::Required>::args(),
                            )
                            .map_err(|err| binrw::error::ContextExt::with_context(
                                err,
                                binrw::error::BacktraceFrame::Full {
                                    message: "While parsing field 'title_name' in XContentHeader"
                                        .into(),
                                    line: 727u32,
                                    file: "stfs/src/parse.rs",
                                    code: Some(
                                        "  ┄────╮\n   724 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mserde\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39mserialize_with = \"serialize_null_wide_string\"\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   725 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mseek_before\u{1b}[39m = \u{1b}[38;5;197mstd\u{1b}[39m::io::SeekFrom::\u{1b}[38;5;148mStart\u{1b}[39m(\u{1b}[38;5;135m0x1691\u{1b}[39m)\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   726 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mdbg\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m727\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub title_name: NullWideString\u{1b}[0m\n  ┄────╯\n",
                                    ),
                                },
                            ))?;
                        {
                            ::std::io::_eprint(
                                format_args!(
                                    "[{0}:{1} | offset {2:#x}] {3} = {4:#x?}\n",
                                    "stfs/src/parse.rs",
                                    727usize,
                                    __binrw_generated_saved_position,
                                    "title_name",
                                    &__binrw_temp,
                                ),
                            );
                        };
                        __binrw_temp
                    };
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut transfer_flags: u8 = {
                    binrw::io::Seek::seek(
                        __binrw_generated_var_reader,
                        std::io::SeekFrom::Start(0x1711),
                    )?;
                    let __binrw_temp = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'transfer_flags' in XContentHeader"
                                    .into(),
                                line: 730u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   729 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mseek_before\u{1b}[39m = \u{1b}[38;5;197mstd\u{1b}[39m::io::SeekFrom::\u{1b}[38;5;148mStart\u{1b}[39m(\u{1b}[38;5;135m0x1711\u{1b}[39m)\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m730\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub transfer_flags: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut thumbnail_image_size: u32 = {
                    let __binrw_generated_saved_position = binrw::io::Seek::stream_position(
                        __binrw_generated_var_reader,
                    )?;
                    let __binrw_temp = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'thumbnail_image_size' in XContentHeader"
                                    .into(),
                                line: 732u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   731 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mdbg\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m732\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub thumbnail_image_size: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    {
                        ::std::io::_eprint(
                            format_args!(
                                "[{0}:{1} | offset {2:#x}] {3} = {4:#x?}\n",
                                "stfs/src/parse.rs",
                                732usize,
                                __binrw_generated_saved_position,
                                "thumbnail_image_size",
                                &__binrw_temp,
                            ),
                        );
                    };
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut title_thumbnail_image_size: u32 = {
                    let __binrw_generated_saved_position = binrw::io::Seek::stream_position(
                        __binrw_generated_var_reader,
                    )?;
                    let __binrw_temp = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'title_thumbnail_image_size' in XContentHeader"
                                    .into(),
                                line: 734u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   733 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mdbg\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m734\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub title_thumbnail_image_size: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    {
                        ::std::io::_eprint(
                            format_args!(
                                "[{0}:{1} | offset {2:#x}] {3} = {4:#x?}\n",
                                "stfs/src/parse.rs",
                                734usize,
                                __binrw_generated_saved_position,
                                "title_thumbnail_image_size",
                                &__binrw_temp,
                            ),
                        );
                    };
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let __binrw_generated_args_thumbnail_image: <Vec<
                    u8,
                > as binrw::BinRead>::Args<'_> = {
                    let args_ty = ::core::marker::PhantomData::<_>;
                    if false {
                        ::binrw::__private::passthrough_helper(args_ty)
                    } else {
                        let builder = ::binrw::__private::builder_helper(args_ty);
                        let builder = builder
                            .count({
                                let __binrw_temp = thumbnail_image_size;
                                #[allow(clippy::useless_conversion)]
                                usize::try_from(__binrw_temp)
                                    .map_err(|_| {
                                        extern crate alloc;
                                        binrw::Error::AssertFail {
                                            pos: binrw::io::Seek::stream_position(
                                                    __binrw_generated_var_reader,
                                                )
                                                .unwrap_or_default(),
                                            message: {
                                                let res = ::alloc::fmt::format(
                                                    format_args!(
                                                        "count {0:?} out of range of usize",
                                                        __binrw_temp,
                                                    ),
                                                );
                                                res
                                            },
                                        }
                                    })?
                            });
                        builder.finalize()
                    }
                };
                let mut thumbnail_image: Vec<u8> = {
                    let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                        __binrw_generated_var_reader,
                    )?;
                    let __binrw_temp = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            __binrw_generated_args_thumbnail_image,
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'thumbnail_image' in XContentHeader"
                                    .into(),
                                line: 738u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   736 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mcount\u{1b}[39m = thumbnail_image_size\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   737 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mpad_size_to\u{1b}[39m(MAX_IMAGE_SIZE)\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m738\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub thumbnail_image: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mVec\u{1b}[39m\u{1b}[0m\u{1b}[1m<\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m>\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    {
                        let pad = (MAX_IMAGE_SIZE) as i64;
                        let size = (binrw::io::Seek::stream_position(
                            __binrw_generated_var_reader,
                        )? - __binrw_generated_position_temp) as i64;
                        if size < pad {
                            binrw::io::Seek::seek(
                                __binrw_generated_var_reader,
                                binrw::io::SeekFrom::Current(pad - size),
                            )?;
                        }
                    }
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let __binrw_generated_args_title_image: <Vec<
                    u8,
                > as binrw::BinRead>::Args<'_> = {
                    let args_ty = ::core::marker::PhantomData::<_>;
                    if false {
                        ::binrw::__private::passthrough_helper(args_ty)
                    } else {
                        let builder = ::binrw::__private::builder_helper(args_ty);
                        let builder = builder
                            .count({
                                let __binrw_temp = title_thumbnail_image_size;
                                #[allow(clippy::useless_conversion)]
                                usize::try_from(__binrw_temp)
                                    .map_err(|_| {
                                        extern crate alloc;
                                        binrw::Error::AssertFail {
                                            pos: binrw::io::Seek::stream_position(
                                                    __binrw_generated_var_reader,
                                                )
                                                .unwrap_or_default(),
                                            message: {
                                                let res = ::alloc::fmt::format(
                                                    format_args!(
                                                        "count {0:?} out of range of usize",
                                                        __binrw_temp,
                                                    ),
                                                );
                                                res
                                            },
                                        }
                                    })?
                            });
                        builder.finalize()
                    }
                };
                let mut title_image: Vec<u8> = {
                    let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                        __binrw_generated_var_reader,
                    )?;
                    let __binrw_temp = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            __binrw_generated_args_title_image,
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'title_image' in XContentHeader"
                                    .into(),
                                line: 742u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   740 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mcount\u{1b}[39m = title_thumbnail_image_size\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   741 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mpad_size_to\u{1b}[39m(MAX_IMAGE_SIZE)\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m742\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub title_image: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mVec\u{1b}[39m\u{1b}[0m\u{1b}[1m<\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m>\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    {
                        let pad = (MAX_IMAGE_SIZE) as i64;
                        let size = (binrw::io::Seek::stream_position(
                            __binrw_generated_var_reader,
                        )? - __binrw_generated_position_temp) as i64;
                        if size < pad {
                            binrw::io::Seek::seek(
                                __binrw_generated_var_reader,
                                binrw::io::SeekFrom::Current(pad - size),
                            )?;
                        }
                    }
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut installer_type: Option<InstallerType> = if ((header_size + 0xFFF)
                    & 0xFFFFF000) - 0x971A > 0x15F4
                {
                    {
                        let __binrw_generated_saved_position = binrw::io::Seek::stream_position(
                            __binrw_generated_var_reader,
                        )?;
                        let __binrw_temp = __binrw_generated_read_function(
                                __binrw_generated_var_reader,
                                __binrw_generated_var_endian,
                                <_ as binrw::__private::Required>::args(),
                            )
                            .map_err(|err| binrw::error::ContextExt::with_context(
                                err,
                                binrw::error::BacktraceFrame::Full {
                                    message: "While parsing field 'installer_type' in XContentHeader"
                                        .into(),
                                    line: 746u32,
                                    file: "stfs/src/parse.rs",
                                    code: Some(
                                        "  ┄────╮\n   744 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mif\u{1b}[39m(((header_size \u{1b}[38;5;197m+\u{1b}[39m \u{1b}[38;5;135m0xFFF\u{1b}[39m) \u{1b}[38;5;197m&\u{1b}[39m \u{1b}[38;5;135m0xFFFFF000\u{1b}[39m) \u{1b}[38;5;197m-\u{1b}[39m \u{1b}[38;5;135m0x971A\u{1b}[39m \u{1b}[38;5;197m>\u{1b}[39m \u{1b}[38;5;135m0x15F4\u{1b}[39m)\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   745 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mdbg\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m746\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpub installer_type: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mOption\u{1b}[39m\u{1b}[0m\u{1b}[1m<InstallerType>\u{1b}[0m\n  ┄────╯\n",
                                    ),
                                },
                            ))?;
                        {
                            ::std::io::_eprint(
                                format_args!(
                                    "[{0}:{1} | offset {2:#x}] {3} = {4:#x?}\n",
                                    "stfs/src/parse.rs",
                                    746usize,
                                    __binrw_generated_saved_position,
                                    "installer_type",
                                    &__binrw_temp,
                                ),
                            );
                        };
                        __binrw_temp
                    }
                } else {
                    <_>::default()
                };
                let __binrw_this = Self {
                    package_type,
                    key_material,
                    license_data,
                    header_hash,
                    header_size,
                    content_type,
                    metadata_version,
                    content_size,
                    media_id,
                    version,
                    base_version,
                    title_id,
                    platform,
                    executable_type,
                    disc_number,
                    disc_in_set,
                    savegame_id,
                    console_id,
                    profile_id,
                    volume_kind,
                    volume_descriptor,
                    data_file_count,
                    data_file_combined_size,
                    device_id,
                    display_name,
                    display_description,
                    publisher_name,
                    title_name,
                    transfer_flags,
                    thumbnail_image_size,
                    title_thumbnail_image_size,
                    thumbnail_image,
                    title_image,
                    installer_type,
                };
                Ok(__binrw_this)
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    impl binrw::meta::ReadEndian for XContentHeader {
        const ENDIAN: binrw::meta::EndianKind = binrw::meta::EndianKind::Endian(
            binrw::Endian::Big,
        );
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for XContentHeader {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_this = self;
            let XContentHeader {
                ref package_type,
                ref key_material,
                ref license_data,
                ref header_hash,
                ref header_size,
                ref content_type,
                ref metadata_version,
                ref content_size,
                ref media_id,
                ref version,
                ref base_version,
                ref title_id,
                ref platform,
                ref executable_type,
                ref disc_number,
                ref disc_in_set,
                ref savegame_id,
                ref console_id,
                ref profile_id,
                ref volume_kind,
                ref volume_descriptor,
                ref data_file_count,
                ref data_file_combined_size,
                ref device_id,
                ref display_name,
                ref display_description,
                ref publisher_name,
                ref title_name,
                ref transfer_flags,
                ref thumbnail_image_size,
                ref title_thumbnail_image_size,
                ref thumbnail_image,
                ref title_image,
                ref installer_type,
            } = self;
            let __binrw_generated_var_endian = binrw::Endian::Big;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_package_type: <PackageType as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &package_type,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_package_type,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_key_material: <KeyMaterial as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &key_material,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_key_material,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_license_data: <[LicenseEntry; 0x10] as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &license_data,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_license_data,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_header_hash: <[u8; 0x14] as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &header_hash,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_header_hash,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_header_size: <u32 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &header_size,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_header_size,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_content_type: <ContentType as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &content_type,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_content_type,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_metadata_version: <u32 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &metadata_version,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_metadata_version,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_content_size: <u64 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &content_size,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_content_size,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_media_id: <u32 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &media_id,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_media_id,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_version: <u32 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &version,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_version,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_base_version: <u32 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &base_version,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_base_version,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_title_id: <u32 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &title_id,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_title_id,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_platform: <u8 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &platform,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_platform,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_executable_type: <u8 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &executable_type,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_executable_type,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_disc_number: <u8 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &disc_number,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_disc_number,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_disc_in_set: <u8 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &disc_in_set,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_disc_in_set,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_savegame_id: <u32 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &savegame_id,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_savegame_id,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_console_id: <[u8; 5] as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &console_id,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_console_id,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_profile_id: <u64 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &profile_id,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_profile_id,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_volume_kind: <FileSystemKind as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            binrw::io::Seek::seek(
                __binrw_generated_var_writer,
                std::io::SeekFrom::Start(0x3a9),
            )?;
            __binrw_generated_write_function(
                &volume_kind,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_volume_kind,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_volume_descriptor: <FileSystem as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            binrw::io::Seek::seek(
                __binrw_generated_var_writer,
                std::io::SeekFrom::Start(0x379),
            )?;
            __binrw_generated_write_function(
                &volume_descriptor,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_volume_descriptor,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_data_file_count: <u32 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &data_file_count,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_data_file_count,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_data_file_combined_size: <u64 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &data_file_combined_size,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_data_file_combined_size,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_device_id: <[u8; 0x14] as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            binrw::io::Seek::seek(
                __binrw_generated_var_writer,
                std::io::SeekFrom::Start(0x3fd),
            )?;
            __binrw_generated_write_function(
                &device_id,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_device_id,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_display_name: <NullWideString as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &display_name,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_display_name,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_display_description: <NullWideString as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            binrw::io::Seek::seek(
                __binrw_generated_var_writer,
                std::io::SeekFrom::Start(0xd11),
            )?;
            __binrw_generated_write_function(
                &display_description,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_display_description,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_publisher_name: <NullWideString as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            binrw::io::Seek::seek(
                __binrw_generated_var_writer,
                std::io::SeekFrom::Start(0x1611),
            )?;
            __binrw_generated_write_function(
                &publisher_name,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_publisher_name,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_title_name: <NullWideString as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            binrw::io::Seek::seek(
                __binrw_generated_var_writer,
                std::io::SeekFrom::Start(0x1691),
            )?;
            __binrw_generated_write_function(
                &title_name,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_title_name,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_transfer_flags: <u8 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            binrw::io::Seek::seek(
                __binrw_generated_var_writer,
                std::io::SeekFrom::Start(0x1711),
            )?;
            __binrw_generated_write_function(
                &transfer_flags,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_transfer_flags,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_thumbnail_image_size: <u32 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &thumbnail_image_size,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_thumbnail_image_size,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_title_thumbnail_image_size: <u32 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &title_thumbnail_image_size,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_title_thumbnail_image_size,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_thumbnail_image: <Vec<
                u8,
            > as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            let __binrw_generated_before_pos = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            __binrw_generated_write_function(
                &thumbnail_image,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_thumbnail_image,
            )?;
            {
                let pad_to_size = (MAX_IMAGE_SIZE) as u64;
                let after_pos = binrw::io::Seek::stream_position(
                    __binrw_generated_var_writer,
                )?;
                if let Some(size) = after_pos.checked_sub(__binrw_generated_before_pos) {
                    if let Some(padding) = pad_to_size.checked_sub(size) {
                        binrw::__private::write_zeroes(
                            __binrw_generated_var_writer,
                            padding,
                        )?;
                    }
                }
            }
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_title_image: <Vec<
                u8,
            > as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            let __binrw_generated_before_pos = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            __binrw_generated_write_function(
                &title_image,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_title_image,
            )?;
            {
                let pad_to_size = (MAX_IMAGE_SIZE) as u64;
                let after_pos = binrw::io::Seek::stream_position(
                    __binrw_generated_var_writer,
                )?;
                if let Some(size) = after_pos.checked_sub(__binrw_generated_before_pos) {
                    if let Some(padding) = pad_to_size.checked_sub(size) {
                        binrw::__private::write_zeroes(
                            __binrw_generated_var_writer,
                            padding,
                        )?;
                    }
                }
            }
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_installer_type: <Option<
                InstallerType,
            > as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &installer_type,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_installer_type,
            )?;
            Ok(())
        }
    }
    impl binrw::meta::WriteEndian for XContentHeader {
        const ENDIAN: binrw::meta::EndianKind = binrw::meta::EndianKind::Endian(
            binrw::Endian::Big,
        );
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for XContentHeader {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            let names: &'static _ = &[
                "package_type",
                "key_material",
                "license_data",
                "header_hash",
                "header_size",
                "content_type",
                "metadata_version",
                "content_size",
                "media_id",
                "version",
                "base_version",
                "title_id",
                "platform",
                "executable_type",
                "disc_number",
                "disc_in_set",
                "savegame_id",
                "console_id",
                "profile_id",
                "volume_kind",
                "volume_descriptor",
                "data_file_count",
                "data_file_combined_size",
                "device_id",
                "display_name",
                "display_description",
                "publisher_name",
                "title_name",
                "transfer_flags",
                "thumbnail_image_size",
                "title_thumbnail_image_size",
                "thumbnail_image",
                "title_image",
                "installer_type",
            ];
            let values: &[&dyn ::core::fmt::Debug] = &[
                &self.package_type,
                &self.key_material,
                &self.license_data,
                &self.header_hash,
                &self.header_size,
                &self.content_type,
                &self.metadata_version,
                &self.content_size,
                &self.media_id,
                &self.version,
                &self.base_version,
                &self.title_id,
                &self.platform,
                &self.executable_type,
                &self.disc_number,
                &self.disc_in_set,
                &self.savegame_id,
                &self.console_id,
                &self.profile_id,
                &self.volume_kind,
                &self.volume_descriptor,
                &self.data_file_count,
                &self.data_file_combined_size,
                &self.device_id,
                &self.display_name,
                &self.display_description,
                &self.publisher_name,
                &self.title_name,
                &self.transfer_flags,
                &self.thumbnail_image_size,
                &self.title_thumbnail_image_size,
                &self.thumbnail_image,
                &self.title_image,
                &&self.installer_type,
            ];
            ::core::fmt::Formatter::debug_struct_fields_finish(
                f,
                "XContentHeader",
                names,
                values,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for XContentHeader {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "XContentHeader",
                    false as usize + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1
                        + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1
                        + 1 + 1 + 1 + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "package_type",
                    &self.package_type,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "key_material",
                    &self.key_material,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "license_data",
                    &self.license_data,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "header_hash",
                    &self.header_hash,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "header_size",
                    &self.header_size,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "content_type",
                    &self.content_type,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "metadata_version",
                    &self.metadata_version,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "content_size",
                    &self.content_size,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "media_id",
                    &self.media_id,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "version",
                    &self.version,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "base_version",
                    &self.base_version,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "title_id",
                    &self.title_id,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "platform",
                    &self.platform,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "executable_type",
                    &self.executable_type,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "disc_number",
                    &self.disc_number,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "disc_in_set",
                    &self.disc_in_set,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "savegame_id",
                    &self.savegame_id,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "console_id",
                    &self.console_id,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "profile_id",
                    &self.profile_id,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "volume_kind",
                    &self.volume_kind,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "volume_descriptor",
                    &self.volume_descriptor,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "data_file_count",
                    &self.data_file_count,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "data_file_combined_size",
                    &self.data_file_combined_size,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "device_id",
                    &self.device_id,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "display_name",
                    {
                        #[doc(hidden)]
                        struct __SerializeWith<'__a> {
                            values: (&'__a NullWideString,),
                            phantom: _serde::__private::PhantomData<XContentHeader>,
                        }
                        impl<'__a> _serde::Serialize for __SerializeWith<'__a> {
                            fn serialize<__S>(
                                &self,
                                __s: __S,
                            ) -> _serde::__private::Result<__S::Ok, __S::Error>
                            where
                                __S: _serde::Serializer,
                            {
                                serialize_null_wide_string(self.values.0, __s)
                            }
                        }
                        &__SerializeWith {
                            values: (&self.display_name,),
                            phantom: _serde::__private::PhantomData::<XContentHeader>,
                        }
                    },
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "display_description",
                    {
                        #[doc(hidden)]
                        struct __SerializeWith<'__a> {
                            values: (&'__a NullWideString,),
                            phantom: _serde::__private::PhantomData<XContentHeader>,
                        }
                        impl<'__a> _serde::Serialize for __SerializeWith<'__a> {
                            fn serialize<__S>(
                                &self,
                                __s: __S,
                            ) -> _serde::__private::Result<__S::Ok, __S::Error>
                            where
                                __S: _serde::Serializer,
                            {
                                serialize_null_wide_string(self.values.0, __s)
                            }
                        }
                        &__SerializeWith {
                            values: (&self.display_description,),
                            phantom: _serde::__private::PhantomData::<XContentHeader>,
                        }
                    },
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "publisher_name",
                    {
                        #[doc(hidden)]
                        struct __SerializeWith<'__a> {
                            values: (&'__a NullWideString,),
                            phantom: _serde::__private::PhantomData<XContentHeader>,
                        }
                        impl<'__a> _serde::Serialize for __SerializeWith<'__a> {
                            fn serialize<__S>(
                                &self,
                                __s: __S,
                            ) -> _serde::__private::Result<__S::Ok, __S::Error>
                            where
                                __S: _serde::Serializer,
                            {
                                serialize_null_wide_string(self.values.0, __s)
                            }
                        }
                        &__SerializeWith {
                            values: (&self.publisher_name,),
                            phantom: _serde::__private::PhantomData::<XContentHeader>,
                        }
                    },
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "title_name",
                    {
                        #[doc(hidden)]
                        struct __SerializeWith<'__a> {
                            values: (&'__a NullWideString,),
                            phantom: _serde::__private::PhantomData<XContentHeader>,
                        }
                        impl<'__a> _serde::Serialize for __SerializeWith<'__a> {
                            fn serialize<__S>(
                                &self,
                                __s: __S,
                            ) -> _serde::__private::Result<__S::Ok, __S::Error>
                            where
                                __S: _serde::Serializer,
                            {
                                serialize_null_wide_string(self.values.0, __s)
                            }
                        }
                        &__SerializeWith {
                            values: (&self.title_name,),
                            phantom: _serde::__private::PhantomData::<XContentHeader>,
                        }
                    },
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "transfer_flags",
                    &self.transfer_flags,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "thumbnail_image_size",
                    &self.thumbnail_image_size,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "title_thumbnail_image_size",
                    &self.title_thumbnail_image_size,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "thumbnail_image",
                    &self.thumbnail_image,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "title_image",
                    &self.title_image,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "installer_type",
                    &self.installer_type,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    impl XContentHeader {
        /// Returns which hash table level the root hash is in
        fn root_hash_table_level(&self) -> Result<HashTableLevel, StfsError> {
            if let FileSystem::Stfs(volume_descriptor) = &self.volume_descriptor {
                let level = if volume_descriptor.allocated_block_count as usize
                    <= HASHES_PER_HASH_TABLE
                {
                    HashTableLevel::First
                } else if volume_descriptor.allocated_block_count as usize
                    <= HASHES_PER_HASH_TABLE_LEVEL[1]
                {
                    HashTableLevel::Second
                } else if volume_descriptor.allocated_block_count as usize
                    <= HASHES_PER_HASH_TABLE_LEVEL[2]
                {
                    HashTableLevel::Third
                } else {
                    return Err(StfsError::InvalidHeader);
                };
                Ok(level)
            } else {
                Err(StfsError::InvalidPackageType)
            }
        }
        pub fn is_read_only(&self) -> bool {
            if let FileSystem::Stfs(stfs) = &self.volume_descriptor {
                stfs.flags.read_only()
            } else {
                false
            }
        }
        pub fn sex(&self) -> StfsPackageSex {
            if self.is_read_only() {
                StfsPackageSex::Male
            } else {
                StfsPackageSex::Female
            }
        }
    }
    pub struct AvatarAssetInformation {
        subcategory: AssetSubcategory,
        colorizable: u32,
        guid: [u8; 0x10],
        skeleton_version: SkeletonVersion,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for AvatarAssetInformation {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut subcategory: AssetSubcategory = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'subcategory' in AvatarAssetInformation"
                                .into(),
                            line: 793u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m793\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1msubcategory: AssetSubcategory\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let __binrw_generated_endian_colorizable = binrw::Endian::Little;
                let mut colorizable: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_endian_colorizable,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'colorizable' in AvatarAssetInformation"
                                .into(),
                            line: 795u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   794 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mlittle\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m795\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mcolorizable: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut guid: [u8; 0x10] = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'guid' in AvatarAssetInformation"
                                .into(),
                            line: 796u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m796\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mguid: [\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m; \u{1b}[0m\u{1b}[1m\u{1b}[38;5;135m0x10\u{1b}[39m\u{1b}[0m\u{1b}[1m]\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut skeleton_version: SkeletonVersion = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'skeleton_version' in AvatarAssetInformation"
                                .into(),
                            line: 797u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m797\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mskeleton_version: SkeletonVersion\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_this = Self {
                    subcategory,
                    colorizable,
                    guid,
                    skeleton_version,
                };
                Ok(__binrw_this)
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for AvatarAssetInformation {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_this = self;
            let AvatarAssetInformation {
                ref subcategory,
                ref colorizable,
                ref guid,
                ref skeleton_version,
            } = self;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_subcategory: <AssetSubcategory as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &subcategory,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_subcategory,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_colorizable: <u32 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &colorizable,
                __binrw_generated_var_writer,
                binrw::Endian::Little,
                __binrw_generated_args_colorizable,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_guid: <[u8; 0x10] as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &guid,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_guid,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_skeleton_version: <SkeletonVersion as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &skeleton_version,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_skeleton_version,
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for AvatarAssetInformation {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field4_finish(
                f,
                "AvatarAssetInformation",
                "subcategory",
                &self.subcategory,
                "colorizable",
                &self.colorizable,
                "guid",
                &self.guid,
                "skeleton_version",
                &&self.skeleton_version,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for AvatarAssetInformation {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "AvatarAssetInformation",
                    false as usize + 1 + 1 + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "subcategory",
                    &self.subcategory,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "colorizable",
                    &self.colorizable,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "guid",
                    &self.guid,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "skeleton_version",
                    &self.skeleton_version,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    pub struct MediaInformation {
        series_id: [u8; 0x10],
        season_id: [u8; 0x10],
        season_number: u16,
        episode_number: u16,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for MediaInformation {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut series_id: [u8; 0x10] = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'series_id' in MediaInformation"
                                .into(),
                            line: 803u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m803\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mseries_id: [\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m; \u{1b}[0m\u{1b}[1m\u{1b}[38;5;135m0x10\u{1b}[39m\u{1b}[0m\u{1b}[1m]\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut season_id: [u8; 0x10] = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'season_id' in MediaInformation"
                                .into(),
                            line: 804u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m804\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mseason_id: [\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m; \u{1b}[0m\u{1b}[1m\u{1b}[38;5;135m0x10\u{1b}[39m\u{1b}[0m\u{1b}[1m]\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut season_number: u16 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'season_number' in MediaInformation"
                                .into(),
                            line: 805u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m805\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mseason_number: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu16\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut episode_number: u16 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'episode_number' in MediaInformation"
                                .into(),
                            line: 806u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m806\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mepisode_number: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu16\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_this = Self {
                    series_id,
                    season_id,
                    season_number,
                    episode_number,
                };
                Ok(__binrw_this)
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for MediaInformation {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_this = self;
            let MediaInformation {
                ref series_id,
                ref season_id,
                ref season_number,
                ref episode_number,
            } = self;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_series_id: <[u8; 0x10] as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &series_id,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_series_id,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_season_id: <[u8; 0x10] as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &season_id,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_season_id,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_season_number: <u16 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &season_number,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_season_number,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_episode_number: <u16 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &episode_number,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_episode_number,
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for MediaInformation {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field4_finish(
                f,
                "MediaInformation",
                "series_id",
                &self.series_id,
                "season_id",
                &self.season_id,
                "season_number",
                &self.season_number,
                "episode_number",
                &&self.episode_number,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for MediaInformation {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "MediaInformation",
                    false as usize + 1 + 1 + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "series_id",
                    &self.series_id,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "season_id",
                    &self.season_id,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "season_number",
                    &self.season_number,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "episode_number",
                    &self.episode_number,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    pub struct InstallerProgressCache {
        resume_state: OnlineContentResumeState,
        current_file_index: u32,
        current_file_offset: u64,
        bytes_processed: u64,
        timestamp_high: u32,
        timestamp_low: u32,
        cab_resume_data: Vec<u8>,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for InstallerProgressCache {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut resume_state: OnlineContentResumeState = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'resume_state' in InstallerProgressCache"
                                .into(),
                            line: 812u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m812\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mresume_state: OnlineContentResumeState\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut current_file_index: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'current_file_index' in InstallerProgressCache"
                                .into(),
                            line: 813u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m813\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mcurrent_file_index: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut current_file_offset: u64 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'current_file_offset' in InstallerProgressCache"
                                .into(),
                            line: 814u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m814\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mcurrent_file_offset: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu64\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut bytes_processed: u64 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'bytes_processed' in InstallerProgressCache"
                                .into(),
                            line: 815u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m815\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mbytes_processed: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu64\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut timestamp_high: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'timestamp_high' in InstallerProgressCache"
                                .into(),
                            line: 816u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m816\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mtimestamp_high: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut timestamp_low: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'timestamp_low' in InstallerProgressCache"
                                .into(),
                            line: 817u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m817\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mtimestamp_low: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let __binrw_generated_args_cab_resume_data: <Vec<
                    u8,
                > as binrw::BinRead>::Args<'_> = {
                    let args_ty = ::core::marker::PhantomData::<_>;
                    if false {
                        ::binrw::__private::passthrough_helper(args_ty)
                    } else {
                        let builder = ::binrw::__private::builder_helper(args_ty);
                        let builder = builder
                            .count({
                                let __binrw_temp = 0;
                                #[allow(clippy::useless_conversion)]
                                usize::try_from(__binrw_temp)
                                    .map_err(|_| {
                                        extern crate alloc;
                                        binrw::Error::AssertFail {
                                            pos: binrw::io::Seek::stream_position(
                                                    __binrw_generated_var_reader,
                                                )
                                                .unwrap_or_default(),
                                            message: {
                                                let res = ::alloc::fmt::format(
                                                    format_args!(
                                                        "count {0:?} out of range of usize",
                                                        __binrw_temp,
                                                    ),
                                                );
                                                res
                                            },
                                        }
                                    })?
                            });
                        builder.finalize()
                    }
                };
                let mut cab_resume_data: Vec<u8> = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        __binrw_generated_args_cab_resume_data,
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'cab_resume_data' in InstallerProgressCache"
                                .into(),
                            line: 819u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   818 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mcount\u{1b}[39m = \u{1b}[38;5;135m0\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m819\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mcab_resume_data: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mVec\u{1b}[39m\u{1b}[0m\u{1b}[1m<\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m>\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_this = Self {
                    resume_state,
                    current_file_index,
                    current_file_offset,
                    bytes_processed,
                    timestamp_high,
                    timestamp_low,
                    cab_resume_data,
                };
                Ok(__binrw_this)
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for InstallerProgressCache {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_this = self;
            let InstallerProgressCache {
                ref resume_state,
                ref current_file_index,
                ref current_file_offset,
                ref bytes_processed,
                ref timestamp_high,
                ref timestamp_low,
                ref cab_resume_data,
            } = self;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_resume_state: <OnlineContentResumeState as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &resume_state,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_resume_state,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_current_file_index: <u32 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &current_file_index,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_current_file_index,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_current_file_offset: <u64 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &current_file_offset,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_current_file_offset,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_bytes_processed: <u64 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &bytes_processed,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_bytes_processed,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_timestamp_high: <u32 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &timestamp_high,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_timestamp_high,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_timestamp_low: <u32 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &timestamp_low,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_timestamp_low,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_cab_resume_data: <Vec<
                u8,
            > as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &cab_resume_data,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_cab_resume_data,
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for InstallerProgressCache {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            let names: &'static _ = &[
                "resume_state",
                "current_file_index",
                "current_file_offset",
                "bytes_processed",
                "timestamp_high",
                "timestamp_low",
                "cab_resume_data",
            ];
            let values: &[&dyn ::core::fmt::Debug] = &[
                &self.resume_state,
                &self.current_file_index,
                &self.current_file_offset,
                &self.bytes_processed,
                &self.timestamp_high,
                &self.timestamp_low,
                &&self.cab_resume_data,
            ];
            ::core::fmt::Formatter::debug_struct_fields_finish(
                f,
                "InstallerProgressCache",
                names,
                values,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for InstallerProgressCache {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "InstallerProgressCache",
                    false as usize + 1 + 1 + 1 + 1 + 1 + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "resume_state",
                    &self.resume_state,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "current_file_index",
                    &self.current_file_index,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "current_file_offset",
                    &self.current_file_offset,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "bytes_processed",
                    &self.bytes_processed,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "timestamp_high",
                    &self.timestamp_high,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "timestamp_low",
                    &self.timestamp_low,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "cab_resume_data",
                    &self.cab_resume_data,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    pub struct FullInstallerMeta {
        installer_base_version: Version,
        installer_version: Version,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for FullInstallerMeta {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut installer_base_version: Version = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'installer_base_version' in FullInstallerMeta"
                                .into(),
                            line: 825u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m825\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1minstaller_base_version: Version\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut installer_version: Version = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'installer_version' in FullInstallerMeta"
                                .into(),
                            line: 826u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m826\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1minstaller_version: Version\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_this = Self {
                    installer_base_version,
                    installer_version,
                };
                Ok(__binrw_this)
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for FullInstallerMeta {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_this = self;
            let FullInstallerMeta {
                ref installer_base_version,
                ref installer_version,
            } = self;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_installer_base_version: <Version as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &installer_base_version,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_installer_base_version,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_installer_version: <Version as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &installer_version,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_installer_version,
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for FullInstallerMeta {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field2_finish(
                f,
                "FullInstallerMeta",
                "installer_base_version",
                &self.installer_base_version,
                "installer_version",
                &&self.installer_version,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for FullInstallerMeta {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "FullInstallerMeta",
                    false as usize + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "installer_base_version",
                    &self.installer_base_version,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "installer_version",
                    &self.installer_version,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    pub enum InstallerMeta {
        FullInstaller(FullInstallerMeta),
        InstallerProgressCache(InstallerProgressCache),
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for InstallerMeta {
        type Args<'__binrw_generated_args_lifetime> = (InstallerType,);
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let (mut installer_type,) = __binrw_generated_var_arguments;
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                extern crate alloc;
                let mut __binrw_generated_error_basket: alloc::vec::Vec<
                    (&'static str, binrw::Error),
                > = alloc::vec::Vec::new();
                match (|| {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    binrw::__private::assert(
                        installer_type.has_full_installer_meta(),
                        __binrw_generated_position_temp,
                        binrw::__private::AssertErrorFn::<
                            _,
                            fn() -> !,
                        >::Message(|| {
                            extern crate alloc;
                            {
                                let res = ::alloc::fmt::format(
                                    format_args!(
                                        "assertion failed: `{0}`",
                                        "installer_type.has_full_installer_meta()",
                                    ),
                                );
                                res
                            }
                        }),
                    )?;
                    let __binrw_generated_read_function = binrw::BinRead::read_options;
                    let mut self_0: FullInstallerMeta = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'self_0' in InstallerMeta::FullInstaller"
                                    .into(),
                                line: 830u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   \u{1b}[1m834\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mFullInstallerMeta\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    let __binrw_this = Self::FullInstaller(self_0);
                    Ok(__binrw_this)
                })() {
                    ok @ Ok(_) => return ok,
                    Err(error) => {
                        binrw::__private::restore_position_variant(
                                __binrw_generated_var_reader,
                                __binrw_generated_position_temp,
                                error,
                            )
                            .map(|__binrw_temp| {
                                __binrw_generated_error_basket
                                    .push(("FullInstaller", __binrw_temp));
                            })?;
                    }
                }
                match (|| {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    binrw::__private::assert(
                        installer_type.has_installer_progress_cache(),
                        __binrw_generated_position_temp,
                        binrw::__private::AssertErrorFn::<
                            _,
                            fn() -> !,
                        >::Message(|| {
                            extern crate alloc;
                            {
                                let res = ::alloc::fmt::format(
                                    format_args!(
                                        "assertion failed: `{0}`",
                                        "installer_type.has_installer_progress_cache()",
                                    ),
                                );
                                res
                            }
                        }),
                    )?;
                    let __binrw_generated_read_function = binrw::BinRead::read_options;
                    let mut self_0: InstallerProgressCache = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'self_0' in InstallerMeta::InstallerProgressCache"
                                    .into(),
                                line: 830u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   \u{1b}[1m836\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mInstallerProgressCache\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    let __binrw_this = Self::InstallerProgressCache(self_0);
                    Ok(__binrw_this)
                })() {
                    ok @ Ok(_) => return ok,
                    Err(error) => {
                        binrw::__private::restore_position_variant(
                                __binrw_generated_var_reader,
                                __binrw_generated_position_temp,
                                error,
                            )
                            .map(|__binrw_temp| {
                                __binrw_generated_error_basket
                                    .push(("InstallerProgressCache", __binrw_temp));
                            })?;
                    }
                }
                Err(binrw::Error::EnumErrors {
                    pos: __binrw_generated_position_temp,
                    variant_errors: __binrw_generated_error_basket,
                })
            })()
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for InstallerMeta {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            match self {
                Self::FullInstaller(ref self_0) => {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                        binrw::BinWrite::write_options,
                    );
                    let __binrw_generated_args_self_0: <FullInstallerMeta as binrw::BinWrite>::Args<
                        '_,
                    > = <_ as binrw::__private::Required>::args();
                    __binrw_generated_write_function(
                        &self_0,
                        __binrw_generated_var_writer,
                        __binrw_generated_var_endian,
                        __binrw_generated_args_self_0,
                    )?;
                }
                Self::InstallerProgressCache(ref self_0) => {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                        binrw::BinWrite::write_options,
                    );
                    let __binrw_generated_args_self_0: <InstallerProgressCache as binrw::BinWrite>::Args<
                        '_,
                    > = <_ as binrw::__private::Required>::args();
                    __binrw_generated_write_function(
                        &self_0,
                        __binrw_generated_var_writer,
                        __binrw_generated_var_endian,
                        __binrw_generated_args_self_0,
                    )?;
                }
            }
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for InstallerMeta {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            match self {
                InstallerMeta::FullInstaller(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "FullInstaller",
                        &__self_0,
                    )
                }
                InstallerMeta::InstallerProgressCache(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "InstallerProgressCache",
                        &__self_0,
                    )
                }
            }
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for InstallerMeta {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    InstallerMeta::FullInstaller(ref __field0) => {
                        _serde::Serializer::serialize_newtype_variant(
                            __serializer,
                            "InstallerMeta",
                            0u32,
                            "FullInstaller",
                            __field0,
                        )
                    }
                    InstallerMeta::InstallerProgressCache(ref __field0) => {
                        _serde::Serializer::serialize_newtype_variant(
                            __serializer,
                            "InstallerMeta",
                            1u32,
                            "InstallerProgressCache",
                            __field0,
                        )
                    }
                }
            }
        }
    };
    impl InstallerMeta {
        pub fn full_installer(self) -> std::option::Option<((FullInstallerMeta))> {
            match self {
                InstallerMeta::FullInstaller(ident_33cc14f346884517a9af9fe44e1351c3) => {
                    std::option::Option::Some(((ident_33cc14f346884517a9af9fe44e1351c3)))
                }
                _ => std::option::Option::None,
            }
        }
        pub fn full_installer_ref(&self) -> std::option::Option<((&FullInstallerMeta))> {
            match self {
                InstallerMeta::FullInstaller(ident_33cc14f346884517a9af9fe44e1351c3) => {
                    std::option::Option::Some(((ident_33cc14f346884517a9af9fe44e1351c3)))
                }
                _ => std::option::Option::None,
            }
        }
        pub fn full_installer_mut(
            &mut self,
        ) -> std::option::Option<((&mut FullInstallerMeta))> {
            match self {
                InstallerMeta::FullInstaller(ident_33cc14f346884517a9af9fe44e1351c3) => {
                    std::option::Option::Some(((ident_33cc14f346884517a9af9fe44e1351c3)))
                }
                _ => std::option::Option::None,
            }
        }
        pub fn full_installer_or<E>(
            self,
            or: E,
        ) -> std::result::Result<((FullInstallerMeta)), E> {
            self.full_installer_or_else(|| or)
        }
        pub fn full_installer_or_else<E, F: std::ops::FnOnce() -> E>(
            self,
            or_else: F,
        ) -> std::result::Result<((FullInstallerMeta)), E> {
            match self {
                InstallerMeta::FullInstaller(ident_33cc14f346884517a9af9fe44e1351c3) => {
                    std::result::Result::Ok(((ident_33cc14f346884517a9af9fe44e1351c3)))
                }
                _ => std::result::Result::Err(or_else()),
            }
        }
        pub fn full_installer_ref_or<E>(
            &self,
            or: E,
        ) -> std::result::Result<((&FullInstallerMeta)), E> {
            self.full_installer_ref_or_else(|| or)
        }
        pub fn full_installer_mut_or<E>(
            &mut self,
            or: E,
        ) -> std::result::Result<((&mut FullInstallerMeta)), E> {
            self.full_installer_mut_or_else(|| or)
        }
        pub fn full_installer_ref_or_else<E, F: std::ops::FnOnce() -> E>(
            &self,
            or_else: F,
        ) -> std::result::Result<((&FullInstallerMeta)), E> {
            match self {
                InstallerMeta::FullInstaller(ident_33cc14f346884517a9af9fe44e1351c3) => {
                    std::result::Result::Ok(((ident_33cc14f346884517a9af9fe44e1351c3)))
                }
                _ => std::result::Result::Err(or_else()),
            }
        }
        pub fn full_installer_mut_or_else<E, F: std::ops::FnOnce() -> E>(
            &mut self,
            or_else: F,
        ) -> std::result::Result<((&mut FullInstallerMeta)), E> {
            match self {
                InstallerMeta::FullInstaller(ident_33cc14f346884517a9af9fe44e1351c3) => {
                    std::result::Result::Ok(((ident_33cc14f346884517a9af9fe44e1351c3)))
                }
                _ => std::result::Result::Err(or_else()),
            }
        }
        pub fn and_then_full_installer<
            F: std::ops::FnOnce(((FullInstallerMeta))) -> ((FullInstallerMeta)),
        >(self, and_then: F) -> Self {
            match self {
                InstallerMeta::FullInstaller(ident_33cc14f346884517a9af9fe44e1351c3) => {
                    let (ident_33cc14f346884517a9af9fe44e1351c3) = and_then(
                        (ident_33cc14f346884517a9af9fe44e1351c3),
                    );
                    InstallerMeta::FullInstaller(ident_33cc14f346884517a9af9fe44e1351c3)
                }
                _ => self,
            }
        }
        pub fn expect_full_installer(self, msg: &str) -> ((FullInstallerMeta)) {
            self.unwrap_or_else_full_installer(|| {
                ::std::rt::panic_display(&msg);
            })
        }
        #[deprecated(
            since = "0.2.0",
            note = "Please use the derived `InstallerMeta::full_installer` method instead. This method will be removed in 1.0.0 or next pre-stable minor bump."
        )]
        pub fn ok_full_installer(self) -> std::option::Option<((FullInstallerMeta))> {
            self.full_installer()
        }
        #[deprecated(
            since = "0.2.0",
            note = "Please use the derived `InstallerMeta::full_installer_or` method instead. This method will be removed in 1.0.0 or next pre-stable minor bump."
        )]
        pub fn ok_or_full_installer<E>(
            self,
            or: E,
        ) -> std::result::Result<((FullInstallerMeta)), E> {
            self.full_installer_or(or)
        }
        #[deprecated(
            since = "0.2.0",
            note = "Please use the derived `InstallerMeta::full_installer_or_else` method instead. This method will be removed in 1.0.0 or next pre-stable minor bump."
        )]
        pub fn ok_or_else_full_installer<E, F: std::ops::FnOnce() -> E>(
            self,
            or_else: F,
        ) -> std::result::Result<((FullInstallerMeta)), E> {
            self.full_installer_or_else(or_else)
        }
        pub fn or_else_full_installer<F: std::ops::FnOnce() -> ((FullInstallerMeta))>(
            self,
            or_else: F,
        ) -> Self {
            match self {
                InstallerMeta::FullInstaller(ident_33cc14f346884517a9af9fe44e1351c3) => {
                    InstallerMeta::FullInstaller(ident_33cc14f346884517a9af9fe44e1351c3)
                }
                _ => {
                    let (ident_33cc14f346884517a9af9fe44e1351c3) = or_else();
                    InstallerMeta::FullInstaller(ident_33cc14f346884517a9af9fe44e1351c3)
                }
            }
        }
        pub fn unwrap_full_installer(self) -> ((FullInstallerMeta)) {
            self.unwrap_or_else_full_installer(|| {
                ::std::rt::begin_panic("explicit panic")
            })
        }
        pub fn unwrap_or_full_installer(
            self,
            or: ((FullInstallerMeta)),
        ) -> ((FullInstallerMeta)) {
            self.unwrap_or_else_full_installer(|| or)
        }
        pub fn unwrap_or_else_full_installer<
            F: std::ops::FnOnce() -> ((FullInstallerMeta)),
        >(self, or_else: F) -> ((FullInstallerMeta)) {
            match self {
                InstallerMeta::FullInstaller(ident_33cc14f346884517a9af9fe44e1351c3) => {
                    ((ident_33cc14f346884517a9af9fe44e1351c3))
                }
                _ => or_else(),
            }
        }
        pub fn is_full_installer(&self) -> bool {
            match self {
                InstallerMeta::FullInstaller(..) => true,
                _ => false,
            }
        }
        pub fn is_not_full_installer(&self) -> bool {
            !self.is_full_installer()
        }
        pub fn and_full_installer(self, and: Self) -> Self {
            match (&self, &and) {
                (InstallerMeta::FullInstaller(..), InstallerMeta::FullInstaller(..)) => {
                    and
                }
                _ => self,
            }
        }
        pub fn or_full_installer(self, or: Self) -> Self {
            match &self {
                InstallerMeta::FullInstaller(..) => self,
                _ => or,
            }
        }
        pub fn installer_progress_cache(
            self,
        ) -> std::option::Option<((InstallerProgressCache))> {
            match self {
                InstallerMeta::InstallerProgressCache(
                    ident_7bc5b9e9900449999156d36cde05f270,
                ) => {
                    std::option::Option::Some(((ident_7bc5b9e9900449999156d36cde05f270)))
                }
                _ => std::option::Option::None,
            }
        }
        pub fn installer_progress_cache_ref(
            &self,
        ) -> std::option::Option<((&InstallerProgressCache))> {
            match self {
                InstallerMeta::InstallerProgressCache(
                    ident_7bc5b9e9900449999156d36cde05f270,
                ) => {
                    std::option::Option::Some(((ident_7bc5b9e9900449999156d36cde05f270)))
                }
                _ => std::option::Option::None,
            }
        }
        pub fn installer_progress_cache_mut(
            &mut self,
        ) -> std::option::Option<((&mut InstallerProgressCache))> {
            match self {
                InstallerMeta::InstallerProgressCache(
                    ident_7bc5b9e9900449999156d36cde05f270,
                ) => {
                    std::option::Option::Some(((ident_7bc5b9e9900449999156d36cde05f270)))
                }
                _ => std::option::Option::None,
            }
        }
        pub fn installer_progress_cache_or<E>(
            self,
            or: E,
        ) -> std::result::Result<((InstallerProgressCache)), E> {
            self.installer_progress_cache_or_else(|| or)
        }
        pub fn installer_progress_cache_or_else<E, F: std::ops::FnOnce() -> E>(
            self,
            or_else: F,
        ) -> std::result::Result<((InstallerProgressCache)), E> {
            match self {
                InstallerMeta::InstallerProgressCache(
                    ident_7bc5b9e9900449999156d36cde05f270,
                ) => std::result::Result::Ok(((ident_7bc5b9e9900449999156d36cde05f270))),
                _ => std::result::Result::Err(or_else()),
            }
        }
        pub fn installer_progress_cache_ref_or<E>(
            &self,
            or: E,
        ) -> std::result::Result<((&InstallerProgressCache)), E> {
            self.installer_progress_cache_ref_or_else(|| or)
        }
        pub fn installer_progress_cache_mut_or<E>(
            &mut self,
            or: E,
        ) -> std::result::Result<((&mut InstallerProgressCache)), E> {
            self.installer_progress_cache_mut_or_else(|| or)
        }
        pub fn installer_progress_cache_ref_or_else<E, F: std::ops::FnOnce() -> E>(
            &self,
            or_else: F,
        ) -> std::result::Result<((&InstallerProgressCache)), E> {
            match self {
                InstallerMeta::InstallerProgressCache(
                    ident_7bc5b9e9900449999156d36cde05f270,
                ) => std::result::Result::Ok(((ident_7bc5b9e9900449999156d36cde05f270))),
                _ => std::result::Result::Err(or_else()),
            }
        }
        pub fn installer_progress_cache_mut_or_else<E, F: std::ops::FnOnce() -> E>(
            &mut self,
            or_else: F,
        ) -> std::result::Result<((&mut InstallerProgressCache)), E> {
            match self {
                InstallerMeta::InstallerProgressCache(
                    ident_7bc5b9e9900449999156d36cde05f270,
                ) => std::result::Result::Ok(((ident_7bc5b9e9900449999156d36cde05f270))),
                _ => std::result::Result::Err(or_else()),
            }
        }
        pub fn and_then_installer_progress_cache<
            F: std::ops::FnOnce(((InstallerProgressCache))) -> ((InstallerProgressCache)),
        >(self, and_then: F) -> Self {
            match self {
                InstallerMeta::InstallerProgressCache(
                    ident_7bc5b9e9900449999156d36cde05f270,
                ) => {
                    let (ident_7bc5b9e9900449999156d36cde05f270) = and_then(
                        (ident_7bc5b9e9900449999156d36cde05f270),
                    );
                    InstallerMeta::InstallerProgressCache(
                        ident_7bc5b9e9900449999156d36cde05f270,
                    )
                }
                _ => self,
            }
        }
        pub fn expect_installer_progress_cache(
            self,
            msg: &str,
        ) -> ((InstallerProgressCache)) {
            self.unwrap_or_else_installer_progress_cache(|| {
                ::std::rt::panic_display(&msg);
            })
        }
        #[deprecated(
            since = "0.2.0",
            note = "Please use the derived `InstallerMeta::installer_progress_cache` method instead. This method will be removed in 1.0.0 or next pre-stable minor bump."
        )]
        pub fn ok_installer_progress_cache(
            self,
        ) -> std::option::Option<((InstallerProgressCache))> {
            self.installer_progress_cache()
        }
        #[deprecated(
            since = "0.2.0",
            note = "Please use the derived `InstallerMeta::installer_progress_cache_or` method instead. This method will be removed in 1.0.0 or next pre-stable minor bump."
        )]
        pub fn ok_or_installer_progress_cache<E>(
            self,
            or: E,
        ) -> std::result::Result<((InstallerProgressCache)), E> {
            self.installer_progress_cache_or(or)
        }
        #[deprecated(
            since = "0.2.0",
            note = "Please use the derived `InstallerMeta::installer_progress_cache_or_else` method instead. This method will be removed in 1.0.0 or next pre-stable minor bump."
        )]
        pub fn ok_or_else_installer_progress_cache<E, F: std::ops::FnOnce() -> E>(
            self,
            or_else: F,
        ) -> std::result::Result<((InstallerProgressCache)), E> {
            self.installer_progress_cache_or_else(or_else)
        }
        pub fn or_else_installer_progress_cache<
            F: std::ops::FnOnce() -> ((InstallerProgressCache)),
        >(self, or_else: F) -> Self {
            match self {
                InstallerMeta::InstallerProgressCache(
                    ident_7bc5b9e9900449999156d36cde05f270,
                ) => {
                    InstallerMeta::InstallerProgressCache(
                        ident_7bc5b9e9900449999156d36cde05f270,
                    )
                }
                _ => {
                    let (ident_7bc5b9e9900449999156d36cde05f270) = or_else();
                    InstallerMeta::InstallerProgressCache(
                        ident_7bc5b9e9900449999156d36cde05f270,
                    )
                }
            }
        }
        pub fn unwrap_installer_progress_cache(self) -> ((InstallerProgressCache)) {
            self.unwrap_or_else_installer_progress_cache(|| {
                ::std::rt::begin_panic("explicit panic")
            })
        }
        pub fn unwrap_or_installer_progress_cache(
            self,
            or: ((InstallerProgressCache)),
        ) -> ((InstallerProgressCache)) {
            self.unwrap_or_else_installer_progress_cache(|| or)
        }
        pub fn unwrap_or_else_installer_progress_cache<
            F: std::ops::FnOnce() -> ((InstallerProgressCache)),
        >(self, or_else: F) -> ((InstallerProgressCache)) {
            match self {
                InstallerMeta::InstallerProgressCache(
                    ident_7bc5b9e9900449999156d36cde05f270,
                ) => ((ident_7bc5b9e9900449999156d36cde05f270)),
                _ => or_else(),
            }
        }
        pub fn is_installer_progress_cache(&self) -> bool {
            match self {
                InstallerMeta::InstallerProgressCache(..) => true,
                _ => false,
            }
        }
        pub fn is_not_installer_progress_cache(&self) -> bool {
            !self.is_installer_progress_cache()
        }
        pub fn and_installer_progress_cache(self, and: Self) -> Self {
            match (&self, &and) {
                (
                    InstallerMeta::InstallerProgressCache(..),
                    InstallerMeta::InstallerProgressCache(..),
                ) => and,
                _ => self,
            }
        }
        pub fn or_installer_progress_cache(self, or: Self) -> Self {
            match &self {
                InstallerMeta::InstallerProgressCache(..) => self,
                _ => or,
            }
        }
    }
    pub struct Certificate {
        pubkey_cert_size: u16,
        owner_console_id: [u8; 5],
        #[serde(serialize_with = "serialize_null_wide_string")]
        owner_console_part_number: NullWideString,
        console_type_flags: Option<ConsoleTypeFlags>,
        date_generation: String,
        public_exponent: u32,
        public_modulus: Vec<u8>,
        certificate_signature: Vec<u8>,
        signature: Vec<u8>,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for Certificate {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut pubkey_cert_size: u16 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'pubkey_cert_size' in Certificate"
                                .into(),
                            line: 842u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m842\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpubkey_cert_size: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu16\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut owner_console_id: [u8; 5] = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'owner_console_id' in Certificate"
                                .into(),
                            line: 843u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m843\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mowner_console_id: [\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m; \u{1b}[0m\u{1b}[1m\u{1b}[38;5;135m5\u{1b}[39m\u{1b}[0m\u{1b}[1m]\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut owner_console_part_number: NullWideString = {
                    let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                        __binrw_generated_var_reader,
                    )?;
                    let __binrw_temp = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'owner_console_part_number' in Certificate"
                                    .into(),
                                line: 846u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   844 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mpad_size_to\u{1b}[39m = \u{1b}[38;5;135m0x11\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   845 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mserde\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39mserialize_with = \"serialize_null_wide_string\"\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m846\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mowner_console_part_number: NullWideString\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    {
                        let pad = (0x11) as i64;
                        let size = (binrw::io::Seek::stream_position(
                            __binrw_generated_var_reader,
                        )? - __binrw_generated_position_temp) as i64;
                        if size < pad {
                            binrw::io::Seek::seek(
                                __binrw_generated_var_reader,
                                binrw::io::SeekFrom::Current(pad - size),
                            )?;
                        }
                    }
                    __binrw_temp
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut console_type_flags: Option<ConsoleTypeFlags> = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'console_type_flags' in Certificate"
                                .into(),
                            line: 847u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m847\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mconsole_type_flags: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mOption\u{1b}[39m\u{1b}[0m\u{1b}[1m<ConsoleTypeFlags>\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut __binrw_generated_map_func_date_generation = (binrw::__private::coerce_fn::<
                    ::core::result::Result<String, _>,
                    _,
                    _,
                >(|x: [u8; 8]| String::from_utf8(x.to_vec())));
                let mut date_generation: String = {
                    let __binrw_generated_saved_position = binrw::io::Seek::stream_position(
                        __binrw_generated_var_reader,
                    )?;
                    __binrw_generated_map_func_date_generation(
                            __binrw_generated_read_function(
                                    __binrw_generated_var_reader,
                                    __binrw_generated_var_endian,
                                    <_ as binrw::__private::Required>::args(),
                                )
                                .map_err(|err| binrw::error::ContextExt::with_context(
                                    err,
                                    binrw::error::BacktraceFrame::Full {
                                        message: "While parsing field 'date_generation' in Certificate"
                                            .into(),
                                        line: 850u32,
                                        file: "stfs/src/parse.rs",
                                        code: Some(
                                            "  ┄────╮\n   848 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mtry_map\u{1b}[39m = |x: [\u{1b}[38;5;197mu8\u{1b}[39m; \u{1b}[38;5;135m8\u{1b}[39m]| \u{1b}[38;5;197mString\u{1b}[39m::\u{1b}[38;5;148mfrom_utf8\u{1b}[39m(x.\u{1b}[38;5;148mto_vec\u{1b}[39m())\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   849 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39mmap = |x| x.as_bytes(), assert(date_generation.len() == 8, \"date_generation.len() != 8\")\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m850\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mdate_generation: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mString\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                                        ),
                                    },
                                ))?,
                        )
                        .map_err(|e| {
                            binrw::Error::Custom {
                                pos: __binrw_generated_saved_position,
                                err: Box::new(e) as _,
                            }
                        })?
                };
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut public_exponent: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'public_exponent' in Certificate"
                                .into(),
                            line: 851u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m851\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpublic_exponent: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let __binrw_generated_args_public_modulus: <Vec<
                    u8,
                > as binrw::BinRead>::Args<'_> = {
                    let args_ty = ::core::marker::PhantomData::<_>;
                    if false {
                        ::binrw::__private::passthrough_helper(args_ty)
                    } else {
                        let builder = ::binrw::__private::builder_helper(args_ty);
                        let builder = builder
                            .count({
                                let __binrw_temp = 0x80;
                                #[allow(clippy::useless_conversion)]
                                usize::try_from(__binrw_temp)
                                    .map_err(|_| {
                                        extern crate alloc;
                                        binrw::Error::AssertFail {
                                            pos: binrw::io::Seek::stream_position(
                                                    __binrw_generated_var_reader,
                                                )
                                                .unwrap_or_default(),
                                            message: {
                                                let res = ::alloc::fmt::format(
                                                    format_args!(
                                                        "count {0:?} out of range of usize",
                                                        __binrw_temp,
                                                    ),
                                                );
                                                res
                                            },
                                        }
                                    })?
                            });
                        builder.finalize()
                    }
                };
                let mut public_modulus: Vec<u8> = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        __binrw_generated_args_public_modulus,
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'public_modulus' in Certificate"
                                .into(),
                            line: 853u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   852 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mcount\u{1b}[39m = \u{1b}[38;5;135m0x80\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m853\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mpublic_modulus: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mVec\u{1b}[39m\u{1b}[0m\u{1b}[1m<\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m>\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let __binrw_generated_args_certificate_signature: <Vec<
                    u8,
                > as binrw::BinRead>::Args<'_> = {
                    let args_ty = ::core::marker::PhantomData::<_>;
                    if false {
                        ::binrw::__private::passthrough_helper(args_ty)
                    } else {
                        let builder = ::binrw::__private::builder_helper(args_ty);
                        let builder = builder
                            .count({
                                let __binrw_temp = 0x100;
                                #[allow(clippy::useless_conversion)]
                                usize::try_from(__binrw_temp)
                                    .map_err(|_| {
                                        extern crate alloc;
                                        binrw::Error::AssertFail {
                                            pos: binrw::io::Seek::stream_position(
                                                    __binrw_generated_var_reader,
                                                )
                                                .unwrap_or_default(),
                                            message: {
                                                let res = ::alloc::fmt::format(
                                                    format_args!(
                                                        "count {0:?} out of range of usize",
                                                        __binrw_temp,
                                                    ),
                                                );
                                                res
                                            },
                                        }
                                    })?
                            });
                        builder.finalize()
                    }
                };
                let mut certificate_signature: Vec<u8> = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        __binrw_generated_args_certificate_signature,
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'certificate_signature' in Certificate"
                                .into(),
                            line: 855u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   854 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mcount\u{1b}[39m = \u{1b}[38;5;135m0x100\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m855\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mcertificate_signature: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mVec\u{1b}[39m\u{1b}[0m\u{1b}[1m<\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m>\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let __binrw_generated_args_signature: <Vec<
                    u8,
                > as binrw::BinRead>::Args<'_> = {
                    let args_ty = ::core::marker::PhantomData::<_>;
                    if false {
                        ::binrw::__private::passthrough_helper(args_ty)
                    } else {
                        let builder = ::binrw::__private::builder_helper(args_ty);
                        let builder = builder
                            .count({
                                let __binrw_temp = 0x80;
                                #[allow(clippy::useless_conversion)]
                                usize::try_from(__binrw_temp)
                                    .map_err(|_| {
                                        extern crate alloc;
                                        binrw::Error::AssertFail {
                                            pos: binrw::io::Seek::stream_position(
                                                    __binrw_generated_var_reader,
                                                )
                                                .unwrap_or_default(),
                                            message: {
                                                let res = ::alloc::fmt::format(
                                                    format_args!(
                                                        "count {0:?} out of range of usize",
                                                        __binrw_temp,
                                                    ),
                                                );
                                                res
                                            },
                                        }
                                    })?
                            });
                        builder.finalize()
                    }
                };
                let mut signature: Vec<u8> = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        __binrw_generated_args_signature,
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'signature' in Certificate"
                                .into(),
                            line: 857u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   856 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mcount\u{1b}[39m = \u{1b}[38;5;135m0x80\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m857\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1msignature: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mVec\u{1b}[39m\u{1b}[0m\u{1b}[1m<\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m>\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_this = Self {
                    pubkey_cert_size,
                    owner_console_id,
                    owner_console_part_number,
                    console_type_flags,
                    date_generation,
                    public_exponent,
                    public_modulus,
                    certificate_signature,
                    signature,
                };
                Ok(__binrw_this)
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for Certificate {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_this = self;
            let Certificate {
                ref pubkey_cert_size,
                ref owner_console_id,
                ref owner_console_part_number,
                ref console_type_flags,
                ref date_generation,
                ref public_exponent,
                ref public_modulus,
                ref certificate_signature,
                ref signature,
            } = self;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_pubkey_cert_size: <u16 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &pubkey_cert_size,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_pubkey_cert_size,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_owner_console_id: <[u8; 5] as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &owner_console_id,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_owner_console_id,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_owner_console_part_number: <NullWideString as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            let __binrw_generated_before_pos = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            __binrw_generated_write_function(
                &owner_console_part_number,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_owner_console_part_number,
            )?;
            {
                let pad_to_size = (0x11) as u64;
                let after_pos = binrw::io::Seek::stream_position(
                    __binrw_generated_var_writer,
                )?;
                if let Some(size) = after_pos.checked_sub(__binrw_generated_before_pos) {
                    if let Some(padding) = pad_to_size.checked_sub(size) {
                        binrw::__private::write_zeroes(
                            __binrw_generated_var_writer,
                            padding,
                        )?;
                    }
                }
            }
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_console_type_flags: <Option<
                ConsoleTypeFlags,
            > as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &console_type_flags,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_console_type_flags,
            )?;
            let __binrw_generated_map_func_date_generation = binrw::__private::write_map_fn_input_type_hint::<
                &String,
                _,
                _,
            >((|x| x.as_bytes()));
            let __binrw_generated_write_function = binrw::__private::write_fn_map_output_type_hint(
                &__binrw_generated_map_func_date_generation,
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_date_generation = binrw::__private::write_map_args_type_hint(
                &__binrw_generated_map_func_date_generation,
                <_ as binrw::__private::Required>::args(),
            );
            binrw::__private::assert(
                date_generation.len() == 8,
                __binrw_generated_position_temp,
                binrw::__private::AssertErrorFn::<
                    _,
                    fn() -> !,
                >::Message(|| {
                    extern crate alloc;
                    {
                        let res = ::alloc::fmt::format(
                            format_args!("date_generation.len() != 8"),
                        );
                        res
                    }
                }),
            )?;
            let date_generation = __binrw_generated_map_func_date_generation(
                date_generation,
            );
            __binrw_generated_write_function(
                &date_generation,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_date_generation,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_public_exponent: <u32 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &public_exponent,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_public_exponent,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_public_modulus: <Vec<
                u8,
            > as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &public_modulus,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_public_modulus,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_certificate_signature: <Vec<
                u8,
            > as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &certificate_signature,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_certificate_signature,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_signature: <Vec<
                u8,
            > as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &signature,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_signature,
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for Certificate {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            let names: &'static _ = &[
                "pubkey_cert_size",
                "owner_console_id",
                "owner_console_part_number",
                "console_type_flags",
                "date_generation",
                "public_exponent",
                "public_modulus",
                "certificate_signature",
                "signature",
            ];
            let values: &[&dyn ::core::fmt::Debug] = &[
                &self.pubkey_cert_size,
                &self.owner_console_id,
                &self.owner_console_part_number,
                &self.console_type_flags,
                &self.date_generation,
                &self.public_exponent,
                &self.public_modulus,
                &self.certificate_signature,
                &&self.signature,
            ];
            ::core::fmt::Formatter::debug_struct_fields_finish(
                f,
                "Certificate",
                names,
                values,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for Certificate {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "Certificate",
                    false as usize + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "pubkey_cert_size",
                    &self.pubkey_cert_size,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "owner_console_id",
                    &self.owner_console_id,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "owner_console_part_number",
                    {
                        #[doc(hidden)]
                        struct __SerializeWith<'__a> {
                            values: (&'__a NullWideString,),
                            phantom: _serde::__private::PhantomData<Certificate>,
                        }
                        impl<'__a> _serde::Serialize for __SerializeWith<'__a> {
                            fn serialize<__S>(
                                &self,
                                __s: __S,
                            ) -> _serde::__private::Result<__S::Ok, __S::Error>
                            where
                                __S: _serde::Serializer,
                            {
                                serialize_null_wide_string(self.values.0, __s)
                            }
                        }
                        &__SerializeWith {
                            values: (&self.owner_console_part_number,),
                            phantom: _serde::__private::PhantomData::<Certificate>,
                        }
                    },
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "console_type_flags",
                    &self.console_type_flags,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "date_generation",
                    &self.date_generation,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "public_exponent",
                    &self.public_exponent,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "public_modulus",
                    &self.public_modulus,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "certificate_signature",
                    &self.certificate_signature,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "signature",
                    &self.signature,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    struct ConsoleTypeFlags {
        bits: u32,
    }
    #[automatically_derived]
    impl ::core::marker::Copy for ConsoleTypeFlags {}
    #[automatically_derived]
    impl ::core::marker::StructuralPartialEq for ConsoleTypeFlags {}
    #[automatically_derived]
    impl ::core::cmp::PartialEq for ConsoleTypeFlags {
        #[inline]
        fn eq(&self, other: &ConsoleTypeFlags) -> bool {
            self.bits == other.bits
        }
    }
    #[automatically_derived]
    impl ::core::cmp::Eq for ConsoleTypeFlags {
        #[inline]
        #[doc(hidden)]
        #[coverage(off)]
        fn assert_receiver_is_total_eq(&self) -> () {
            let _: ::core::cmp::AssertParamIsEq<u32>;
        }
    }
    #[automatically_derived]
    impl ::core::clone::Clone for ConsoleTypeFlags {
        #[inline]
        fn clone(&self) -> ConsoleTypeFlags {
            let _: ::core::clone::AssertParamIsClone<u32>;
            *self
        }
    }
    #[automatically_derived]
    impl ::core::cmp::PartialOrd for ConsoleTypeFlags {
        #[inline]
        fn partial_cmp(
            &self,
            other: &ConsoleTypeFlags,
        ) -> ::core::option::Option<::core::cmp::Ordering> {
            ::core::cmp::PartialOrd::partial_cmp(&self.bits, &other.bits)
        }
    }
    #[automatically_derived]
    impl ::core::cmp::Ord for ConsoleTypeFlags {
        #[inline]
        fn cmp(&self, other: &ConsoleTypeFlags) -> ::core::cmp::Ordering {
            ::core::cmp::Ord::cmp(&self.bits, &other.bits)
        }
    }
    #[automatically_derived]
    impl ::core::hash::Hash for ConsoleTypeFlags {
        #[inline]
        fn hash<__H: ::core::hash::Hasher>(&self, state: &mut __H) -> () {
            ::core::hash::Hash::hash(&self.bits, state)
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for ConsoleTypeFlags {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut bits: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'bits' in ConsoleTypeFlags"
                                .into(),
                            line: 860u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m365\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mbits: $T\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_this = Self { bits };
                Ok(__binrw_this)
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for ConsoleTypeFlags {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_this = self;
            let ConsoleTypeFlags { ref bits } = self;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_bits: <u32 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &bits,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_bits,
            )?;
            Ok(())
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for ConsoleTypeFlags {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "ConsoleTypeFlags",
                    false as usize + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "bits",
                    &self.bits,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    impl ::bitflags::_core::fmt::Debug for ConsoleTypeFlags {
        fn fmt(
            &self,
            f: &mut ::bitflags::_core::fmt::Formatter,
        ) -> ::bitflags::_core::fmt::Result {
            #[allow(non_snake_case)]
            trait __BitFlags {
                #[inline]
                fn DEVKIT(&self) -> bool {
                    false
                }
                #[inline]
                fn RETAIL(&self) -> bool {
                    false
                }
                #[inline]
                fn TESTKIT(&self) -> bool {
                    false
                }
                #[inline]
                fn RECOVERY_GENERATED(&self) -> bool {
                    false
                }
            }
            #[allow(non_snake_case)]
            impl __BitFlags for ConsoleTypeFlags {
                #[allow(deprecated)]
                #[inline]
                fn DEVKIT(&self) -> bool {
                    if Self::DEVKIT.bits == 0 && self.bits != 0 {
                        false
                    } else {
                        self.bits & Self::DEVKIT.bits == Self::DEVKIT.bits
                    }
                }
                #[allow(deprecated)]
                #[inline]
                fn RETAIL(&self) -> bool {
                    if Self::RETAIL.bits == 0 && self.bits != 0 {
                        false
                    } else {
                        self.bits & Self::RETAIL.bits == Self::RETAIL.bits
                    }
                }
                #[allow(deprecated)]
                #[inline]
                fn TESTKIT(&self) -> bool {
                    if Self::TESTKIT.bits == 0 && self.bits != 0 {
                        false
                    } else {
                        self.bits & Self::TESTKIT.bits == Self::TESTKIT.bits
                    }
                }
                #[allow(deprecated)]
                #[inline]
                fn RECOVERY_GENERATED(&self) -> bool {
                    if Self::RECOVERY_GENERATED.bits == 0 && self.bits != 0 {
                        false
                    } else {
                        self.bits & Self::RECOVERY_GENERATED.bits
                            == Self::RECOVERY_GENERATED.bits
                    }
                }
            }
            let mut first = true;
            if <Self as __BitFlags>::DEVKIT(self) {
                if !first {
                    f.write_str(" | ")?;
                }
                first = false;
                f.write_str("DEVKIT")?;
            }
            if <Self as __BitFlags>::RETAIL(self) {
                if !first {
                    f.write_str(" | ")?;
                }
                first = false;
                f.write_str("RETAIL")?;
            }
            if <Self as __BitFlags>::TESTKIT(self) {
                if !first {
                    f.write_str(" | ")?;
                }
                first = false;
                f.write_str("TESTKIT")?;
            }
            if <Self as __BitFlags>::RECOVERY_GENERATED(self) {
                if !first {
                    f.write_str(" | ")?;
                }
                first = false;
                f.write_str("RECOVERY_GENERATED")?;
            }
            let extra_bits = self.bits & !Self::all().bits();
            if extra_bits != 0 {
                if !first {
                    f.write_str(" | ")?;
                }
                first = false;
                f.write_str("0x")?;
                ::bitflags::_core::fmt::LowerHex::fmt(&extra_bits, f)?;
            }
            if first {
                f.write_str("(empty)")?;
            }
            Ok(())
        }
    }
    impl ::bitflags::_core::fmt::Binary for ConsoleTypeFlags {
        fn fmt(
            &self,
            f: &mut ::bitflags::_core::fmt::Formatter,
        ) -> ::bitflags::_core::fmt::Result {
            ::bitflags::_core::fmt::Binary::fmt(&self.bits, f)
        }
    }
    impl ::bitflags::_core::fmt::Octal for ConsoleTypeFlags {
        fn fmt(
            &self,
            f: &mut ::bitflags::_core::fmt::Formatter,
        ) -> ::bitflags::_core::fmt::Result {
            ::bitflags::_core::fmt::Octal::fmt(&self.bits, f)
        }
    }
    impl ::bitflags::_core::fmt::LowerHex for ConsoleTypeFlags {
        fn fmt(
            &self,
            f: &mut ::bitflags::_core::fmt::Formatter,
        ) -> ::bitflags::_core::fmt::Result {
            ::bitflags::_core::fmt::LowerHex::fmt(&self.bits, f)
        }
    }
    impl ::bitflags::_core::fmt::UpperHex for ConsoleTypeFlags {
        fn fmt(
            &self,
            f: &mut ::bitflags::_core::fmt::Formatter,
        ) -> ::bitflags::_core::fmt::Result {
            ::bitflags::_core::fmt::UpperHex::fmt(&self.bits, f)
        }
    }
    #[allow(dead_code)]
    impl ConsoleTypeFlags {
        pub const DEVKIT: Self = Self { bits: 0x1 };
        pub const RETAIL: Self = Self { bits: 0x2 };
        pub const TESTKIT: Self = Self { bits: 0x40000000 };
        pub const RECOVERY_GENERATED: Self = Self { bits: 0x80000000 };
        /// Returns an empty set of flags.
        #[inline]
        pub const fn empty() -> Self {
            Self { bits: 0 }
        }
        /// Returns the set containing all flags.
        #[inline]
        pub const fn all() -> Self {
            #[allow(non_snake_case)]
            trait __BitFlags {
                const DEVKIT: u32 = 0;
                const RETAIL: u32 = 0;
                const TESTKIT: u32 = 0;
                const RECOVERY_GENERATED: u32 = 0;
            }
            #[allow(non_snake_case)]
            impl __BitFlags for ConsoleTypeFlags {
                #[allow(deprecated)]
                const DEVKIT: u32 = Self::DEVKIT.bits;
                #[allow(deprecated)]
                const RETAIL: u32 = Self::RETAIL.bits;
                #[allow(deprecated)]
                const TESTKIT: u32 = Self::TESTKIT.bits;
                #[allow(deprecated)]
                const RECOVERY_GENERATED: u32 = Self::RECOVERY_GENERATED.bits;
            }
            Self {
                bits: <Self as __BitFlags>::DEVKIT | <Self as __BitFlags>::RETAIL
                    | <Self as __BitFlags>::TESTKIT
                    | <Self as __BitFlags>::RECOVERY_GENERATED,
            }
        }
        /// Returns the raw value of the flags currently stored.
        #[inline]
        pub const fn bits(&self) -> u32 {
            self.bits
        }
        /// Convert from underlying bit representation, unless that
        /// representation contains bits that do not correspond to a flag.
        #[inline]
        pub const fn from_bits(bits: u32) -> ::bitflags::_core::option::Option<Self> {
            if (bits & !Self::all().bits()) == 0 {
                ::bitflags::_core::option::Option::Some(Self { bits })
            } else {
                ::bitflags::_core::option::Option::None
            }
        }
        /// Convert from underlying bit representation, dropping any bits
        /// that do not correspond to flags.
        #[inline]
        pub const fn from_bits_truncate(bits: u32) -> Self {
            Self {
                bits: bits & Self::all().bits,
            }
        }
        /// Convert from underlying bit representation, preserving all
        /// bits (even those not corresponding to a defined flag).
        ///
        /// # Safety
        ///
        /// The caller of the `bitflags!` macro can chose to allow or
        /// disallow extra bits for their bitflags type.
        ///
        /// The caller of `from_bits_unchecked()` has to ensure that
        /// all bits correspond to a defined flag or that extra bits
        /// are valid for this bitflags type.
        #[inline]
        pub const unsafe fn from_bits_unchecked(bits: u32) -> Self {
            Self { bits }
        }
        /// Returns `true` if no flags are currently stored.
        #[inline]
        pub const fn is_empty(&self) -> bool {
            self.bits() == Self::empty().bits()
        }
        /// Returns `true` if all flags are currently set.
        #[inline]
        pub const fn is_all(&self) -> bool {
            Self::all().bits | self.bits == self.bits
        }
        /// Returns `true` if there are flags common to both `self` and `other`.
        #[inline]
        pub const fn intersects(&self, other: Self) -> bool {
            !(Self {
                bits: self.bits & other.bits,
            })
                .is_empty()
        }
        /// Returns `true` if all of the flags in `other` are contained within `self`.
        #[inline]
        pub const fn contains(&self, other: Self) -> bool {
            (self.bits & other.bits) == other.bits
        }
        /// Inserts the specified flags in-place.
        #[inline]
        pub fn insert(&mut self, other: Self) {
            self.bits |= other.bits;
        }
        /// Removes the specified flags in-place.
        #[inline]
        pub fn remove(&mut self, other: Self) {
            self.bits &= !other.bits;
        }
        /// Toggles the specified flags in-place.
        #[inline]
        pub fn toggle(&mut self, other: Self) {
            self.bits ^= other.bits;
        }
        /// Inserts or removes the specified flags depending on the passed value.
        #[inline]
        pub fn set(&mut self, other: Self, value: bool) {
            if value {
                self.insert(other);
            } else {
                self.remove(other);
            }
        }
        /// Returns the intersection between the flags in `self` and
        /// `other`.
        ///
        /// Specifically, the returned set contains only the flags which are
        /// present in *both* `self` *and* `other`.
        ///
        /// This is equivalent to using the `&` operator (e.g.
        /// [`ops::BitAnd`]), as in `flags & other`.
        ///
        /// [`ops::BitAnd`]: https://doc.rust-lang.org/std/ops/trait.BitAnd.html
        #[inline]
        #[must_use]
        pub const fn intersection(self, other: Self) -> Self {
            Self {
                bits: self.bits & other.bits,
            }
        }
        /// Returns the union of between the flags in `self` and `other`.
        ///
        /// Specifically, the returned set contains all flags which are
        /// present in *either* `self` *or* `other`, including any which are
        /// present in both (see [`Self::symmetric_difference`] if that
        /// is undesirable).
        ///
        /// This is equivalent to using the `|` operator (e.g.
        /// [`ops::BitOr`]), as in `flags | other`.
        ///
        /// [`ops::BitOr`]: https://doc.rust-lang.org/std/ops/trait.BitOr.html
        #[inline]
        #[must_use]
        pub const fn union(self, other: Self) -> Self {
            Self {
                bits: self.bits | other.bits,
            }
        }
        /// Returns the difference between the flags in `self` and `other`.
        ///
        /// Specifically, the returned set contains all flags present in
        /// `self`, except for the ones present in `other`.
        ///
        /// It is also conceptually equivalent to the "bit-clear" operation:
        /// `flags & !other` (and this syntax is also supported).
        ///
        /// This is equivalent to using the `-` operator (e.g.
        /// [`ops::Sub`]), as in `flags - other`.
        ///
        /// [`ops::Sub`]: https://doc.rust-lang.org/std/ops/trait.Sub.html
        #[inline]
        #[must_use]
        pub const fn difference(self, other: Self) -> Self {
            Self {
                bits: self.bits & !other.bits,
            }
        }
        /// Returns the [symmetric difference][sym-diff] between the flags
        /// in `self` and `other`.
        ///
        /// Specifically, the returned set contains the flags present which
        /// are present in `self` or `other`, but that are not present in
        /// both. Equivalently, it contains the flags present in *exactly
        /// one* of the sets `self` and `other`.
        ///
        /// This is equivalent to using the `^` operator (e.g.
        /// [`ops::BitXor`]), as in `flags ^ other`.
        ///
        /// [sym-diff]: https://en.wikipedia.org/wiki/Symmetric_difference
        /// [`ops::BitXor`]: https://doc.rust-lang.org/std/ops/trait.BitXor.html
        #[inline]
        #[must_use]
        pub const fn symmetric_difference(self, other: Self) -> Self {
            Self {
                bits: self.bits ^ other.bits,
            }
        }
        /// Returns the complement of this set of flags.
        ///
        /// Specifically, the returned set contains all the flags which are
        /// not set in `self`, but which are allowed for this type.
        ///
        /// Alternatively, it can be thought of as the set difference
        /// between [`Self::all()`] and `self` (e.g. `Self::all() - self`)
        ///
        /// This is equivalent to using the `!` operator (e.g.
        /// [`ops::Not`]), as in `!flags`.
        ///
        /// [`Self::all()`]: Self::all
        /// [`ops::Not`]: https://doc.rust-lang.org/std/ops/trait.Not.html
        #[inline]
        #[must_use]
        pub const fn complement(self) -> Self {
            Self::from_bits_truncate(!self.bits)
        }
    }
    impl ::bitflags::_core::ops::BitOr for ConsoleTypeFlags {
        type Output = Self;
        /// Returns the union of the two sets of flags.
        #[inline]
        fn bitor(self, other: ConsoleTypeFlags) -> Self {
            Self {
                bits: self.bits | other.bits,
            }
        }
    }
    impl ::bitflags::_core::ops::BitOrAssign for ConsoleTypeFlags {
        /// Adds the set of flags.
        #[inline]
        fn bitor_assign(&mut self, other: Self) {
            self.bits |= other.bits;
        }
    }
    impl ::bitflags::_core::ops::BitXor for ConsoleTypeFlags {
        type Output = Self;
        /// Returns the left flags, but with all the right flags toggled.
        #[inline]
        fn bitxor(self, other: Self) -> Self {
            Self {
                bits: self.bits ^ other.bits,
            }
        }
    }
    impl ::bitflags::_core::ops::BitXorAssign for ConsoleTypeFlags {
        /// Toggles the set of flags.
        #[inline]
        fn bitxor_assign(&mut self, other: Self) {
            self.bits ^= other.bits;
        }
    }
    impl ::bitflags::_core::ops::BitAnd for ConsoleTypeFlags {
        type Output = Self;
        /// Returns the intersection between the two sets of flags.
        #[inline]
        fn bitand(self, other: Self) -> Self {
            Self {
                bits: self.bits & other.bits,
            }
        }
    }
    impl ::bitflags::_core::ops::BitAndAssign for ConsoleTypeFlags {
        /// Disables all flags disabled in the set.
        #[inline]
        fn bitand_assign(&mut self, other: Self) {
            self.bits &= other.bits;
        }
    }
    impl ::bitflags::_core::ops::Sub for ConsoleTypeFlags {
        type Output = Self;
        /// Returns the set difference of the two sets of flags.
        #[inline]
        fn sub(self, other: Self) -> Self {
            Self {
                bits: self.bits & !other.bits,
            }
        }
    }
    impl ::bitflags::_core::ops::SubAssign for ConsoleTypeFlags {
        /// Disables all flags enabled in the set.
        #[inline]
        fn sub_assign(&mut self, other: Self) {
            self.bits &= !other.bits;
        }
    }
    impl ::bitflags::_core::ops::Not for ConsoleTypeFlags {
        type Output = Self;
        /// Returns the complement of this set of flags.
        #[inline]
        fn not(self) -> Self {
            Self { bits: !self.bits } & Self::all()
        }
    }
    impl ::bitflags::_core::iter::Extend<ConsoleTypeFlags> for ConsoleTypeFlags {
        fn extend<T: ::bitflags::_core::iter::IntoIterator<Item = Self>>(
            &mut self,
            iterator: T,
        ) {
            for item in iterator {
                self.insert(item)
            }
        }
    }
    impl ::bitflags::_core::iter::FromIterator<ConsoleTypeFlags> for ConsoleTypeFlags {
        fn from_iter<T: ::bitflags::_core::iter::IntoIterator<Item = Self>>(
            iterator: T,
        ) -> Self {
            let mut result = Self::empty();
            result.extend(iterator);
            result
        }
    }
    enum LicenseType {
        Unused = 0x0000,
        Unrestricted = 0xFFFF,
        ConsoleProfileLicense = 0x0009,
        WindowsProfileLicense = 0x0003,
        ConsoleLicense = 0xF000,
        MediaFlags = 0xE000,
        KeyVaultPrivileges = 0xD000,
        HyperVisorFlags = 0xC000,
        UserPrivileges = 0xB000,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for LicenseType {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_temp: u16 = binrw::BinRead::read_options(
                    __binrw_generated_var_reader,
                    __binrw_generated_var_endian,
                    (),
                )?;
                if __binrw_temp == Self::Unused as u16 {
                    Ok(Self::Unused)
                } else if __binrw_temp == Self::Unrestricted as u16 {
                    Ok(Self::Unrestricted)
                } else if __binrw_temp == Self::ConsoleProfileLicense as u16 {
                    Ok(Self::ConsoleProfileLicense)
                } else if __binrw_temp == Self::WindowsProfileLicense as u16 {
                    Ok(Self::WindowsProfileLicense)
                } else if __binrw_temp == Self::ConsoleLicense as u16 {
                    Ok(Self::ConsoleLicense)
                } else if __binrw_temp == Self::MediaFlags as u16 {
                    Ok(Self::MediaFlags)
                } else if __binrw_temp == Self::KeyVaultPrivileges as u16 {
                    Ok(Self::KeyVaultPrivileges)
                } else if __binrw_temp == Self::HyperVisorFlags as u16 {
                    Ok(Self::HyperVisorFlags)
                } else if __binrw_temp == Self::UserPrivileges as u16 {
                    Ok(Self::UserPrivileges)
                } else {
                    Err(
                        binrw::error::ContextExt::with_context(
                            binrw::Error::NoVariantMatch {
                                pos: __binrw_generated_position_temp,
                            },
                            binrw::error::BacktraceFrame::Message({
                                extern crate alloc;
                                {
                                    let res = ::alloc::fmt::format(
                                        format_args!(
                                            "Unexpected value for enum: {0:?}",
                                            __binrw_temp,
                                        ),
                                    );
                                    res
                                }
                                    .into()
                            }),
                        ),
                    )
                }
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for LicenseType {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            binrw::BinWrite::write_options(
                &(match self {
                    Self::Unused => Self::Unused,
                    Self::Unrestricted => Self::Unrestricted,
                    Self::ConsoleProfileLicense => Self::ConsoleProfileLicense,
                    Self::WindowsProfileLicense => Self::WindowsProfileLicense,
                    Self::ConsoleLicense => Self::ConsoleLicense,
                    Self::MediaFlags => Self::MediaFlags,
                    Self::KeyVaultPrivileges => Self::KeyVaultPrivileges,
                    Self::HyperVisorFlags => Self::HyperVisorFlags,
                    Self::UserPrivileges => Self::UserPrivileges,
                } as u16),
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                (),
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for LicenseType {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    LicenseType::Unused => "Unused",
                    LicenseType::Unrestricted => "Unrestricted",
                    LicenseType::ConsoleProfileLicense => "ConsoleProfileLicense",
                    LicenseType::WindowsProfileLicense => "WindowsProfileLicense",
                    LicenseType::ConsoleLicense => "ConsoleLicense",
                    LicenseType::MediaFlags => "MediaFlags",
                    LicenseType::KeyVaultPrivileges => "KeyVaultPrivileges",
                    LicenseType::HyperVisorFlags => "HyperVisorFlags",
                    LicenseType::UserPrivileges => "UserPrivileges",
                },
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for LicenseType {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    LicenseType::Unused => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "LicenseType",
                            0u32,
                            "Unused",
                        )
                    }
                    LicenseType::Unrestricted => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "LicenseType",
                            1u32,
                            "Unrestricted",
                        )
                    }
                    LicenseType::ConsoleProfileLicense => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "LicenseType",
                            2u32,
                            "ConsoleProfileLicense",
                        )
                    }
                    LicenseType::WindowsProfileLicense => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "LicenseType",
                            3u32,
                            "WindowsProfileLicense",
                        )
                    }
                    LicenseType::ConsoleLicense => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "LicenseType",
                            4u32,
                            "ConsoleLicense",
                        )
                    }
                    LicenseType::MediaFlags => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "LicenseType",
                            5u32,
                            "MediaFlags",
                        )
                    }
                    LicenseType::KeyVaultPrivileges => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "LicenseType",
                            6u32,
                            "KeyVaultPrivileges",
                        )
                    }
                    LicenseType::HyperVisorFlags => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "LicenseType",
                            7u32,
                            "HyperVisorFlags",
                        )
                    }
                    LicenseType::UserPrivileges => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "LicenseType",
                            8u32,
                            "UserPrivileges",
                        )
                    }
                }
            }
        }
    };
    #[automatically_derived]
    impl ::core::clone::Clone for LicenseType {
        #[inline]
        fn clone(&self) -> LicenseType {
            *self
        }
    }
    #[automatically_derived]
    impl ::core::marker::Copy for LicenseType {}
    impl Default for LicenseType {
        fn default() -> Self {
            Self::Unused
        }
    }
    pub struct LicenseEntry {
        ty: LicenseType,
        data: [u8; 6],
        bits: u32,
        flags: u32,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for LicenseEntry {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut ty: LicenseType = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'ty' in LicenseEntry".into(),
                            line: 895u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m895\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mty: LicenseType\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut data: [u8; 6] = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'data' in LicenseEntry".into(),
                            line: 896u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m896\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mdata: [\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m; \u{1b}[0m\u{1b}[1m\u{1b}[38;5;135m6\u{1b}[39m\u{1b}[0m\u{1b}[1m]\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut bits: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'bits' in LicenseEntry".into(),
                            line: 897u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m897\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mbits: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut flags: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'flags' in LicenseEntry"
                                .into(),
                            line: 898u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄────╮\n   \u{1b}[1m898\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mflags: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_this = Self { ty, data, bits, flags };
                Ok(__binrw_this)
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for LicenseEntry {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_this = self;
            let LicenseEntry { ref ty, ref data, ref bits, ref flags } = self;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_ty: <LicenseType as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &ty,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_ty,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_data: <[u8; 6] as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &data,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_data,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_bits: <u32 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &bits,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_bits,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_flags: <u32 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &flags,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_flags,
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::default::Default for LicenseEntry {
        #[inline]
        fn default() -> LicenseEntry {
            LicenseEntry {
                ty: ::core::default::Default::default(),
                data: ::core::default::Default::default(),
                bits: ::core::default::Default::default(),
                flags: ::core::default::Default::default(),
            }
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for LicenseEntry {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field4_finish(
                f,
                "LicenseEntry",
                "ty",
                &self.ty,
                "data",
                &self.data,
                "bits",
                &self.bits,
                "flags",
                &&self.flags,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for LicenseEntry {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "LicenseEntry",
                    false as usize + 1 + 1 + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "ty",
                    &self.ty,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "data",
                    &self.data,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "bits",
                    &self.bits,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "flags",
                    &self.flags,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    pub enum ContentMetadata {
        AvatarItem(AvatarAssetInformation),
        Video(MediaInformation),
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for ContentMetadata {
        type Args<'__binrw_generated_args_lifetime> = (ContentType,);
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let (mut content_type,) = __binrw_generated_var_arguments;
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                extern crate alloc;
                let mut __binrw_generated_error_basket: alloc::vec::Vec<
                    (&'static str, binrw::Error),
                > = alloc::vec::Vec::new();
                match (|| {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    binrw::__private::assert(
                        content_type == ContentType::AvatarItem,
                        __binrw_generated_position_temp,
                        binrw::__private::AssertErrorFn::<
                            _,
                            fn() -> !,
                        >::Message(|| {
                            extern crate alloc;
                            {
                                let res = ::alloc::fmt::format(
                                    format_args!(
                                        "assertion failed: `{0}`",
                                        "content_type == ContentType :: AvatarItem",
                                    ),
                                );
                                res
                            }
                        }),
                    )?;
                    let __binrw_generated_read_function = binrw::BinRead::read_options;
                    let mut self_0: AvatarAssetInformation = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'self_0' in ContentMetadata::AvatarItem"
                                    .into(),
                                line: 902u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   \u{1b}[1m906\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mAvatarAssetInformation\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    let __binrw_this = Self::AvatarItem(self_0);
                    Ok(__binrw_this)
                })() {
                    ok @ Ok(_) => return ok,
                    Err(error) => {
                        binrw::__private::restore_position_variant(
                                __binrw_generated_var_reader,
                                __binrw_generated_position_temp,
                                error,
                            )
                            .map(|__binrw_temp| {
                                __binrw_generated_error_basket
                                    .push(("AvatarItem", __binrw_temp));
                            })?;
                    }
                }
                match (|| {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    binrw::__private::assert(
                        content_type == ContentType::Video,
                        __binrw_generated_position_temp,
                        binrw::__private::AssertErrorFn::<
                            _,
                            fn() -> !,
                        >::Message(|| {
                            extern crate alloc;
                            {
                                let res = ::alloc::fmt::format(
                                    format_args!(
                                        "assertion failed: `{0}`",
                                        "content_type == ContentType :: Video",
                                    ),
                                );
                                res
                            }
                        }),
                    )?;
                    let __binrw_generated_read_function = binrw::BinRead::read_options;
                    let mut self_0: MediaInformation = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'self_0' in ContentMetadata::Video"
                                    .into(),
                                line: 902u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄────╮\n   \u{1b}[1m909\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mMediaInformation\u{1b}[0m\n  ┄────╯\n",
                                ),
                            },
                        ))?;
                    let __binrw_this = Self::Video(self_0);
                    Ok(__binrw_this)
                })() {
                    ok @ Ok(_) => return ok,
                    Err(error) => {
                        binrw::__private::restore_position_variant(
                                __binrw_generated_var_reader,
                                __binrw_generated_position_temp,
                                error,
                            )
                            .map(|__binrw_temp| {
                                __binrw_generated_error_basket
                                    .push(("Video", __binrw_temp));
                            })?;
                    }
                }
                Err(binrw::Error::EnumErrors {
                    pos: __binrw_generated_position_temp,
                    variant_errors: __binrw_generated_error_basket,
                })
            })()
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for ContentMetadata {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            match self {
                Self::AvatarItem(ref self_0) => {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                        binrw::BinWrite::write_options,
                    );
                    let __binrw_generated_args_self_0: <AvatarAssetInformation as binrw::BinWrite>::Args<
                        '_,
                    > = <_ as binrw::__private::Required>::args();
                    __binrw_generated_write_function(
                        &self_0,
                        __binrw_generated_var_writer,
                        __binrw_generated_var_endian,
                        __binrw_generated_args_self_0,
                    )?;
                }
                Self::Video(ref self_0) => {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                        binrw::BinWrite::write_options,
                    );
                    let __binrw_generated_args_self_0: <MediaInformation as binrw::BinWrite>::Args<
                        '_,
                    > = <_ as binrw::__private::Required>::args();
                    __binrw_generated_write_function(
                        &self_0,
                        __binrw_generated_var_writer,
                        __binrw_generated_var_endian,
                        __binrw_generated_args_self_0,
                    )?;
                }
            }
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for ContentMetadata {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            match self {
                ContentMetadata::AvatarItem(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "AvatarItem",
                        &__self_0,
                    )
                }
                ContentMetadata::Video(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "Video",
                        &__self_0,
                    )
                }
            }
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for ContentMetadata {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    ContentMetadata::AvatarItem(ref __field0) => {
                        _serde::Serializer::serialize_newtype_variant(
                            __serializer,
                            "ContentMetadata",
                            0u32,
                            "AvatarItem",
                            __field0,
                        )
                    }
                    ContentMetadata::Video(ref __field0) => {
                        _serde::Serializer::serialize_newtype_variant(
                            __serializer,
                            "ContentMetadata",
                            1u32,
                            "Video",
                            __field0,
                        )
                    }
                }
            }
        }
    };
    pub enum ContentType {
        ArcadeGame = 0xD0000,
        AvatarAssetPack = 0x8000,
        AvatarItem = 0x9000,
        CacheFile = 0x40000,
        CommunityGame = 0x2000000,
        GameDemo = 0x80000,
        GameOnDemand = 0x7000,
        GamerPicture = 0x20000,
        GamerTitle = 0xA0000,
        GameTrailer = 0xC0000,
        GameVideo = 0x400000,
        InstalledGame = 0x4000,
        Installer = 0xB0000,
        IPTVPauseBuffer = 0x2000,
        LicenseStore = 0xF0000,
        MarketPlaceContent = 2,
        Movie = 0x100000,
        MusicVideo = 0x300000,
        PodcastVideo = 0x500000,
        Profile = 0x10000,
        Publisher = 3,
        SavedGame = 1,
        StorageDownload = 0x50000,
        Theme = 0x30000,
        Video = 0x200000,
        ViralVideo = 0x600000,
        XboxDownload = 0x70000,
        XboxOriginalGame = 0x5000,
        XboxSavedGame = 0x60000,
        Xbox360Title = 0x1000,
        XNA = 0xE0000,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for ContentType {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_temp: u32 = binrw::BinRead::read_options(
                    __binrw_generated_var_reader,
                    __binrw_generated_var_endian,
                    (),
                )?;
                if __binrw_temp == Self::ArcadeGame as u32 {
                    Ok(Self::ArcadeGame)
                } else if __binrw_temp == Self::AvatarAssetPack as u32 {
                    Ok(Self::AvatarAssetPack)
                } else if __binrw_temp == Self::AvatarItem as u32 {
                    Ok(Self::AvatarItem)
                } else if __binrw_temp == Self::CacheFile as u32 {
                    Ok(Self::CacheFile)
                } else if __binrw_temp == Self::CommunityGame as u32 {
                    Ok(Self::CommunityGame)
                } else if __binrw_temp == Self::GameDemo as u32 {
                    Ok(Self::GameDemo)
                } else if __binrw_temp == Self::GameOnDemand as u32 {
                    Ok(Self::GameOnDemand)
                } else if __binrw_temp == Self::GamerPicture as u32 {
                    Ok(Self::GamerPicture)
                } else if __binrw_temp == Self::GamerTitle as u32 {
                    Ok(Self::GamerTitle)
                } else if __binrw_temp == Self::GameTrailer as u32 {
                    Ok(Self::GameTrailer)
                } else if __binrw_temp == Self::GameVideo as u32 {
                    Ok(Self::GameVideo)
                } else if __binrw_temp == Self::InstalledGame as u32 {
                    Ok(Self::InstalledGame)
                } else if __binrw_temp == Self::Installer as u32 {
                    Ok(Self::Installer)
                } else if __binrw_temp == Self::IPTVPauseBuffer as u32 {
                    Ok(Self::IPTVPauseBuffer)
                } else if __binrw_temp == Self::LicenseStore as u32 {
                    Ok(Self::LicenseStore)
                } else if __binrw_temp == Self::MarketPlaceContent as u32 {
                    Ok(Self::MarketPlaceContent)
                } else if __binrw_temp == Self::Movie as u32 {
                    Ok(Self::Movie)
                } else if __binrw_temp == Self::MusicVideo as u32 {
                    Ok(Self::MusicVideo)
                } else if __binrw_temp == Self::PodcastVideo as u32 {
                    Ok(Self::PodcastVideo)
                } else if __binrw_temp == Self::Profile as u32 {
                    Ok(Self::Profile)
                } else if __binrw_temp == Self::Publisher as u32 {
                    Ok(Self::Publisher)
                } else if __binrw_temp == Self::SavedGame as u32 {
                    Ok(Self::SavedGame)
                } else if __binrw_temp == Self::StorageDownload as u32 {
                    Ok(Self::StorageDownload)
                } else if __binrw_temp == Self::Theme as u32 {
                    Ok(Self::Theme)
                } else if __binrw_temp == Self::Video as u32 {
                    Ok(Self::Video)
                } else if __binrw_temp == Self::ViralVideo as u32 {
                    Ok(Self::ViralVideo)
                } else if __binrw_temp == Self::XboxDownload as u32 {
                    Ok(Self::XboxDownload)
                } else if __binrw_temp == Self::XboxOriginalGame as u32 {
                    Ok(Self::XboxOriginalGame)
                } else if __binrw_temp == Self::XboxSavedGame as u32 {
                    Ok(Self::XboxSavedGame)
                } else if __binrw_temp == Self::Xbox360Title as u32 {
                    Ok(Self::Xbox360Title)
                } else if __binrw_temp == Self::XNA as u32 {
                    Ok(Self::XNA)
                } else {
                    Err(
                        binrw::error::ContextExt::with_context(
                            binrw::Error::NoVariantMatch {
                                pos: __binrw_generated_position_temp,
                            },
                            binrw::error::BacktraceFrame::Message({
                                extern crate alloc;
                                {
                                    let res = ::alloc::fmt::format(
                                        format_args!(
                                            "Unexpected value for enum: {0:?}",
                                            __binrw_temp,
                                        ),
                                    );
                                    res
                                }
                                    .into()
                            }),
                        ),
                    )
                }
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for ContentType {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            binrw::BinWrite::write_options(
                &(match self {
                    Self::ArcadeGame => Self::ArcadeGame,
                    Self::AvatarAssetPack => Self::AvatarAssetPack,
                    Self::AvatarItem => Self::AvatarItem,
                    Self::CacheFile => Self::CacheFile,
                    Self::CommunityGame => Self::CommunityGame,
                    Self::GameDemo => Self::GameDemo,
                    Self::GameOnDemand => Self::GameOnDemand,
                    Self::GamerPicture => Self::GamerPicture,
                    Self::GamerTitle => Self::GamerTitle,
                    Self::GameTrailer => Self::GameTrailer,
                    Self::GameVideo => Self::GameVideo,
                    Self::InstalledGame => Self::InstalledGame,
                    Self::Installer => Self::Installer,
                    Self::IPTVPauseBuffer => Self::IPTVPauseBuffer,
                    Self::LicenseStore => Self::LicenseStore,
                    Self::MarketPlaceContent => Self::MarketPlaceContent,
                    Self::Movie => Self::Movie,
                    Self::MusicVideo => Self::MusicVideo,
                    Self::PodcastVideo => Self::PodcastVideo,
                    Self::Profile => Self::Profile,
                    Self::Publisher => Self::Publisher,
                    Self::SavedGame => Self::SavedGame,
                    Self::StorageDownload => Self::StorageDownload,
                    Self::Theme => Self::Theme,
                    Self::Video => Self::Video,
                    Self::ViralVideo => Self::ViralVideo,
                    Self::XboxDownload => Self::XboxDownload,
                    Self::XboxOriginalGame => Self::XboxOriginalGame,
                    Self::XboxSavedGame => Self::XboxSavedGame,
                    Self::Xbox360Title => Self::Xbox360Title,
                    Self::XNA => Self::XNA,
                } as u32),
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                (),
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for ContentType {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    ContentType::ArcadeGame => "ArcadeGame",
                    ContentType::AvatarAssetPack => "AvatarAssetPack",
                    ContentType::AvatarItem => "AvatarItem",
                    ContentType::CacheFile => "CacheFile",
                    ContentType::CommunityGame => "CommunityGame",
                    ContentType::GameDemo => "GameDemo",
                    ContentType::GameOnDemand => "GameOnDemand",
                    ContentType::GamerPicture => "GamerPicture",
                    ContentType::GamerTitle => "GamerTitle",
                    ContentType::GameTrailer => "GameTrailer",
                    ContentType::GameVideo => "GameVideo",
                    ContentType::InstalledGame => "InstalledGame",
                    ContentType::Installer => "Installer",
                    ContentType::IPTVPauseBuffer => "IPTVPauseBuffer",
                    ContentType::LicenseStore => "LicenseStore",
                    ContentType::MarketPlaceContent => "MarketPlaceContent",
                    ContentType::Movie => "Movie",
                    ContentType::MusicVideo => "MusicVideo",
                    ContentType::PodcastVideo => "PodcastVideo",
                    ContentType::Profile => "Profile",
                    ContentType::Publisher => "Publisher",
                    ContentType::SavedGame => "SavedGame",
                    ContentType::StorageDownload => "StorageDownload",
                    ContentType::Theme => "Theme",
                    ContentType::Video => "Video",
                    ContentType::ViralVideo => "ViralVideo",
                    ContentType::XboxDownload => "XboxDownload",
                    ContentType::XboxOriginalGame => "XboxOriginalGame",
                    ContentType::XboxSavedGame => "XboxSavedGame",
                    ContentType::Xbox360Title => "Xbox360Title",
                    ContentType::XNA => "XNA",
                },
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for ContentType {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    ContentType::ArcadeGame => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            0u32,
                            "ArcadeGame",
                        )
                    }
                    ContentType::AvatarAssetPack => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            1u32,
                            "AvatarAssetPack",
                        )
                    }
                    ContentType::AvatarItem => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            2u32,
                            "AvatarItem",
                        )
                    }
                    ContentType::CacheFile => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            3u32,
                            "CacheFile",
                        )
                    }
                    ContentType::CommunityGame => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            4u32,
                            "CommunityGame",
                        )
                    }
                    ContentType::GameDemo => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            5u32,
                            "GameDemo",
                        )
                    }
                    ContentType::GameOnDemand => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            6u32,
                            "GameOnDemand",
                        )
                    }
                    ContentType::GamerPicture => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            7u32,
                            "GamerPicture",
                        )
                    }
                    ContentType::GamerTitle => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            8u32,
                            "GamerTitle",
                        )
                    }
                    ContentType::GameTrailer => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            9u32,
                            "GameTrailer",
                        )
                    }
                    ContentType::GameVideo => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            10u32,
                            "GameVideo",
                        )
                    }
                    ContentType::InstalledGame => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            11u32,
                            "InstalledGame",
                        )
                    }
                    ContentType::Installer => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            12u32,
                            "Installer",
                        )
                    }
                    ContentType::IPTVPauseBuffer => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            13u32,
                            "IPTVPauseBuffer",
                        )
                    }
                    ContentType::LicenseStore => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            14u32,
                            "LicenseStore",
                        )
                    }
                    ContentType::MarketPlaceContent => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            15u32,
                            "MarketPlaceContent",
                        )
                    }
                    ContentType::Movie => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            16u32,
                            "Movie",
                        )
                    }
                    ContentType::MusicVideo => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            17u32,
                            "MusicVideo",
                        )
                    }
                    ContentType::PodcastVideo => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            18u32,
                            "PodcastVideo",
                        )
                    }
                    ContentType::Profile => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            19u32,
                            "Profile",
                        )
                    }
                    ContentType::Publisher => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            20u32,
                            "Publisher",
                        )
                    }
                    ContentType::SavedGame => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            21u32,
                            "SavedGame",
                        )
                    }
                    ContentType::StorageDownload => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            22u32,
                            "StorageDownload",
                        )
                    }
                    ContentType::Theme => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            23u32,
                            "Theme",
                        )
                    }
                    ContentType::Video => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            24u32,
                            "Video",
                        )
                    }
                    ContentType::ViralVideo => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            25u32,
                            "ViralVideo",
                        )
                    }
                    ContentType::XboxDownload => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            26u32,
                            "XboxDownload",
                        )
                    }
                    ContentType::XboxOriginalGame => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            27u32,
                            "XboxOriginalGame",
                        )
                    }
                    ContentType::XboxSavedGame => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            28u32,
                            "XboxSavedGame",
                        )
                    }
                    ContentType::Xbox360Title => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            29u32,
                            "Xbox360Title",
                        )
                    }
                    ContentType::XNA => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "ContentType",
                            30u32,
                            "XNA",
                        )
                    }
                }
            }
        }
    };
    #[automatically_derived]
    impl ::core::marker::Copy for ContentType {}
    #[automatically_derived]
    impl ::core::clone::Clone for ContentType {
        #[inline]
        fn clone(&self) -> ContentType {
            *self
        }
    }
    #[automatically_derived]
    impl ::core::marker::StructuralPartialEq for ContentType {}
    #[automatically_derived]
    impl ::core::cmp::PartialEq for ContentType {
        #[inline]
        fn eq(&self, other: &ContentType) -> bool {
            let __self_tag = ::core::intrinsics::discriminant_value(self);
            let __arg1_tag = ::core::intrinsics::discriminant_value(other);
            __self_tag == __arg1_tag
        }
    }
    #[automatically_derived]
    impl ::core::cmp::Eq for ContentType {
        #[inline]
        #[doc(hidden)]
        #[coverage(off)]
        fn assert_receiver_is_total_eq(&self) -> () {}
    }
    #[automatically_derived]
    impl ::core::cmp::PartialOrd for ContentType {
        #[inline]
        fn partial_cmp(
            &self,
            other: &ContentType,
        ) -> ::core::option::Option<::core::cmp::Ordering> {
            let __self_tag = ::core::intrinsics::discriminant_value(self);
            let __arg1_tag = ::core::intrinsics::discriminant_value(other);
            ::core::cmp::PartialOrd::partial_cmp(&__self_tag, &__arg1_tag)
        }
    }
    #[automatically_derived]
    impl ::core::cmp::Ord for ContentType {
        #[inline]
        fn cmp(&self, other: &ContentType) -> ::core::cmp::Ordering {
            let __self_tag = ::core::intrinsics::discriminant_value(self);
            let __arg1_tag = ::core::intrinsics::discriminant_value(other);
            ::core::cmp::Ord::cmp(&__self_tag, &__arg1_tag)
        }
    }
    impl ContentType {
        pub fn has_content_metadata(&self) -> bool {
            match self {
                ContentType::AvatarItem | ContentType::Video => true,
                _ => false,
            }
        }
    }
    pub enum InstallerType {
        None = 0,
        SystemUpdate = 0x53555044,
        TitleUpdate = 0x54555044,
        SystemUpdateProgressCache = 0x50245355,
        TitleUpdateProgressCache = 0x50245455,
        TitleContentProgressCache = 0x50245443,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for InstallerType {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_temp: u32 = binrw::BinRead::read_options(
                    __binrw_generated_var_reader,
                    __binrw_generated_var_endian,
                    (),
                )?;
                if __binrw_temp == Self::None as u32 {
                    Ok(Self::None)
                } else if __binrw_temp == Self::SystemUpdate as u32 {
                    Ok(Self::SystemUpdate)
                } else if __binrw_temp == Self::TitleUpdate as u32 {
                    Ok(Self::TitleUpdate)
                } else if __binrw_temp == Self::SystemUpdateProgressCache as u32 {
                    Ok(Self::SystemUpdateProgressCache)
                } else if __binrw_temp == Self::TitleUpdateProgressCache as u32 {
                    Ok(Self::TitleUpdateProgressCache)
                } else if __binrw_temp == Self::TitleContentProgressCache as u32 {
                    Ok(Self::TitleContentProgressCache)
                } else {
                    Err(
                        binrw::error::ContextExt::with_context(
                            binrw::Error::NoVariantMatch {
                                pos: __binrw_generated_position_temp,
                            },
                            binrw::error::BacktraceFrame::Message({
                                extern crate alloc;
                                {
                                    let res = ::alloc::fmt::format(
                                        format_args!(
                                            "Unexpected value for enum: {0:?}",
                                            __binrw_temp,
                                        ),
                                    );
                                    res
                                }
                                    .into()
                            }),
                        ),
                    )
                }
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for InstallerType {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            binrw::BinWrite::write_options(
                &(match self {
                    Self::None => Self::None,
                    Self::SystemUpdate => Self::SystemUpdate,
                    Self::TitleUpdate => Self::TitleUpdate,
                    Self::SystemUpdateProgressCache => Self::SystemUpdateProgressCache,
                    Self::TitleUpdateProgressCache => Self::TitleUpdateProgressCache,
                    Self::TitleContentProgressCache => Self::TitleContentProgressCache,
                } as u32),
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                (),
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for InstallerType {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    InstallerType::None => "None",
                    InstallerType::SystemUpdate => "SystemUpdate",
                    InstallerType::TitleUpdate => "TitleUpdate",
                    InstallerType::SystemUpdateProgressCache => {
                        "SystemUpdateProgressCache"
                    }
                    InstallerType::TitleUpdateProgressCache => "TitleUpdateProgressCache",
                    InstallerType::TitleContentProgressCache => {
                        "TitleContentProgressCache"
                    }
                },
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for InstallerType {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    InstallerType::None => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "InstallerType",
                            0u32,
                            "None",
                        )
                    }
                    InstallerType::SystemUpdate => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "InstallerType",
                            1u32,
                            "SystemUpdate",
                        )
                    }
                    InstallerType::TitleUpdate => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "InstallerType",
                            2u32,
                            "TitleUpdate",
                        )
                    }
                    InstallerType::SystemUpdateProgressCache => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "InstallerType",
                            3u32,
                            "SystemUpdateProgressCache",
                        )
                    }
                    InstallerType::TitleUpdateProgressCache => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "InstallerType",
                            4u32,
                            "TitleUpdateProgressCache",
                        )
                    }
                    InstallerType::TitleContentProgressCache => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "InstallerType",
                            5u32,
                            "TitleContentProgressCache",
                        )
                    }
                }
            }
        }
    };
    #[automatically_derived]
    impl ::core::marker::Copy for InstallerType {}
    #[automatically_derived]
    impl ::core::clone::Clone for InstallerType {
        #[inline]
        fn clone(&self) -> InstallerType {
            *self
        }
    }
    impl InstallerType {
        pub fn has_full_installer_meta(&self) -> bool {
            match self {
                InstallerType::SystemUpdate | InstallerType::TitleUpdate => true,
                _ => false,
            }
        }
        pub fn has_installer_progress_cache(&self) -> bool {
            match self {
                InstallerType::SystemUpdateProgressCache
                | InstallerType::TitleUpdateProgressCache
                | Self::TitleContentProgressCache => true,
                _ => false,
            }
        }
    }
    pub struct Version {
        major: u16,
        minor: u16,
        build: u16,
        revision: u16,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for Version {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                binrw::BinRead::read_options(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        (),
                    )
                    .map(|input: u32| Self::from(input))
                    .and_then(|__binrw_this| {
                        let Self { ref major, ref minor, ref build, ref revision } = &__binrw_this;
                        (|| { Ok(()) })().map(|_: ()| __binrw_this)
                    })
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    impl binrw::meta::ReadEndian for Version {
        const ENDIAN: binrw::meta::EndianKind = binrw::meta::EndianKind::None;
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for Version {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            binrw::BinWrite::write_options(
                &((|this: &Self| u32::from(*this))(self)),
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                (),
            )?;
            Ok(())
        }
    }
    impl binrw::meta::WriteEndian for Version {
        const ENDIAN: binrw::meta::EndianKind = binrw::meta::EndianKind::None;
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for Version {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field4_finish(
                f,
                "Version",
                "major",
                &self.major,
                "minor",
                &self.minor,
                "build",
                &self.build,
                "revision",
                &&self.revision,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for Version {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "Version",
                    false as usize + 1 + 1 + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "major",
                    &self.major,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "minor",
                    &self.minor,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "build",
                    &self.build,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "revision",
                    &self.revision,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    #[automatically_derived]
    impl ::core::marker::Copy for Version {}
    #[automatically_derived]
    impl ::core::clone::Clone for Version {
        #[inline]
        fn clone(&self) -> Version {
            let _: ::core::clone::AssertParamIsClone<u16>;
            *self
        }
    }
    impl From<u32> for Version {
        fn from(input: u32) -> Self {
            Version {
                major: ((input & 0xF000_0000) >> 28) as u16,
                minor: ((input & 0x0F00_0000) >> 24) as u16,
                build: ((input & 0x00FF_FF00) >> 8) as u16,
                revision: (input & 0xFF) as u16,
            }
        }
    }
    impl From<Version> for u32 {
        fn from(value: Version) -> Self {
            let Version { major, minor, build, revision } = value;
            let major = major as u32;
            let minor = minor as u32;
            let build = build as u32;
            let revision = revision as u32;
            (major << 28) | (minor << 24) | (build << 8) | revision
        }
    }
    enum OnlineContentResumeState {
        FileHeadersNotReady = 0x46494C48,
        NewFolder = 0x666F6C64,
        NewFolderResumeAttempt1 = 0x666F6C31,
        NewFolderResumeAttempt2 = 0x666F6C32,
        NewFolderResumeAttemptUnknown = 0x666F6C3F,
        NewFolderResumeAttemptSpecific = 0x666F6C40,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for OnlineContentResumeState {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_temp: u32 = binrw::BinRead::read_options(
                    __binrw_generated_var_reader,
                    __binrw_generated_var_endian,
                    (),
                )?;
                if __binrw_temp == Self::FileHeadersNotReady as u32 {
                    Ok(Self::FileHeadersNotReady)
                } else if __binrw_temp == Self::NewFolder as u32 {
                    Ok(Self::NewFolder)
                } else if __binrw_temp == Self::NewFolderResumeAttempt1 as u32 {
                    Ok(Self::NewFolderResumeAttempt1)
                } else if __binrw_temp == Self::NewFolderResumeAttempt2 as u32 {
                    Ok(Self::NewFolderResumeAttempt2)
                } else if __binrw_temp == Self::NewFolderResumeAttemptUnknown as u32 {
                    Ok(Self::NewFolderResumeAttemptUnknown)
                } else if __binrw_temp == Self::NewFolderResumeAttemptSpecific as u32 {
                    Ok(Self::NewFolderResumeAttemptSpecific)
                } else {
                    Err(
                        binrw::error::ContextExt::with_context(
                            binrw::Error::NoVariantMatch {
                                pos: __binrw_generated_position_temp,
                            },
                            binrw::error::BacktraceFrame::Message({
                                extern crate alloc;
                                {
                                    let res = ::alloc::fmt::format(
                                        format_args!(
                                            "Unexpected value for enum: {0:?}",
                                            __binrw_temp,
                                        ),
                                    );
                                    res
                                }
                                    .into()
                            }),
                        ),
                    )
                }
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for OnlineContentResumeState {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            binrw::BinWrite::write_options(
                &(match self {
                    Self::FileHeadersNotReady => Self::FileHeadersNotReady,
                    Self::NewFolder => Self::NewFolder,
                    Self::NewFolderResumeAttempt1 => Self::NewFolderResumeAttempt1,
                    Self::NewFolderResumeAttempt2 => Self::NewFolderResumeAttempt2,
                    Self::NewFolderResumeAttemptUnknown => {
                        Self::NewFolderResumeAttemptUnknown
                    }
                    Self::NewFolderResumeAttemptSpecific => {
                        Self::NewFolderResumeAttemptSpecific
                    }
                } as u32),
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                (),
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for OnlineContentResumeState {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    OnlineContentResumeState::FileHeadersNotReady => {
                        "FileHeadersNotReady"
                    }
                    OnlineContentResumeState::NewFolder => "NewFolder",
                    OnlineContentResumeState::NewFolderResumeAttempt1 => {
                        "NewFolderResumeAttempt1"
                    }
                    OnlineContentResumeState::NewFolderResumeAttempt2 => {
                        "NewFolderResumeAttempt2"
                    }
                    OnlineContentResumeState::NewFolderResumeAttemptUnknown => {
                        "NewFolderResumeAttemptUnknown"
                    }
                    OnlineContentResumeState::NewFolderResumeAttemptSpecific => {
                        "NewFolderResumeAttemptSpecific"
                    }
                },
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for OnlineContentResumeState {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    OnlineContentResumeState::FileHeadersNotReady => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "OnlineContentResumeState",
                            0u32,
                            "FileHeadersNotReady",
                        )
                    }
                    OnlineContentResumeState::NewFolder => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "OnlineContentResumeState",
                            1u32,
                            "NewFolder",
                        )
                    }
                    OnlineContentResumeState::NewFolderResumeAttempt1 => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "OnlineContentResumeState",
                            2u32,
                            "NewFolderResumeAttempt1",
                        )
                    }
                    OnlineContentResumeState::NewFolderResumeAttempt2 => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "OnlineContentResumeState",
                            3u32,
                            "NewFolderResumeAttempt2",
                        )
                    }
                    OnlineContentResumeState::NewFolderResumeAttemptUnknown => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "OnlineContentResumeState",
                            4u32,
                            "NewFolderResumeAttemptUnknown",
                        )
                    }
                    OnlineContentResumeState::NewFolderResumeAttemptSpecific => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "OnlineContentResumeState",
                            5u32,
                            "NewFolderResumeAttemptSpecific",
                        )
                    }
                }
            }
        }
    };
    #[automatically_derived]
    impl ::core::marker::Copy for OnlineContentResumeState {}
    #[automatically_derived]
    impl ::core::clone::Clone for OnlineContentResumeState {
        #[inline]
        fn clone(&self) -> OnlineContentResumeState {
            *self
        }
    }
    pub enum XContentFlags {
        MetadataIsPEC = 1,
        MetadataSkipRead = 2,
        MetadataDontFreeThumbnails = 4,
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for XContentFlags {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    XContentFlags::MetadataIsPEC => "MetadataIsPEC",
                    XContentFlags::MetadataSkipRead => "MetadataSkipRead",
                    XContentFlags::MetadataDontFreeThumbnails => {
                        "MetadataDontFreeThumbnails"
                    }
                },
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for XContentFlags {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    XContentFlags::MetadataIsPEC => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "XContentFlags",
                            0u32,
                            "MetadataIsPEC",
                        )
                    }
                    XContentFlags::MetadataSkipRead => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "XContentFlags",
                            1u32,
                            "MetadataSkipRead",
                        )
                    }
                    XContentFlags::MetadataDontFreeThumbnails => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "XContentFlags",
                            2u32,
                            "MetadataDontFreeThumbnails",
                        )
                    }
                }
            }
        }
    };
    #[automatically_derived]
    impl ::core::marker::Copy for XContentFlags {}
    #[automatically_derived]
    impl ::core::clone::Clone for XContentFlags {
        #[inline]
        fn clone(&self) -> XContentFlags {
            *self
        }
    }
    pub enum FileSystemKind {
        Stfs = 0,
        Svod,
        Fatx,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for FileSystemKind {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_temp: u32 = binrw::BinRead::read_options(
                    __binrw_generated_var_reader,
                    __binrw_generated_var_endian,
                    (),
                )?;
                if __binrw_temp == Self::Stfs as u32 {
                    Ok(Self::Stfs)
                } else if __binrw_temp == Self::Svod as u32 {
                    Ok(Self::Svod)
                } else if __binrw_temp == Self::Fatx as u32 {
                    Ok(Self::Fatx)
                } else {
                    Err(
                        binrw::error::ContextExt::with_context(
                            binrw::Error::NoVariantMatch {
                                pos: __binrw_generated_position_temp,
                            },
                            binrw::error::BacktraceFrame::Message({
                                extern crate alloc;
                                {
                                    let res = ::alloc::fmt::format(
                                        format_args!(
                                            "Unexpected value for enum: {0:?}",
                                            __binrw_temp,
                                        ),
                                    );
                                    res
                                }
                                    .into()
                            }),
                        ),
                    )
                }
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for FileSystemKind {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            binrw::BinWrite::write_options(
                &(match self {
                    Self::Stfs => Self::Stfs,
                    Self::Svod => Self::Svod,
                    Self::Fatx => Self::Fatx,
                } as u32),
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                (),
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for FileSystemKind {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    FileSystemKind::Stfs => "Stfs",
                    FileSystemKind::Svod => "Svod",
                    FileSystemKind::Fatx => "Fatx",
                },
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for FileSystemKind {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    FileSystemKind::Stfs => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "FileSystemKind",
                            0u32,
                            "Stfs",
                        )
                    }
                    FileSystemKind::Svod => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "FileSystemKind",
                            1u32,
                            "Svod",
                        )
                    }
                    FileSystemKind::Fatx => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "FileSystemKind",
                            2u32,
                            "Fatx",
                        )
                    }
                }
            }
        }
    };
    #[automatically_derived]
    impl ::core::marker::StructuralPartialEq for FileSystemKind {}
    #[automatically_derived]
    impl ::core::cmp::PartialEq for FileSystemKind {
        #[inline]
        fn eq(&self, other: &FileSystemKind) -> bool {
            let __self_tag = ::core::intrinsics::discriminant_value(self);
            let __arg1_tag = ::core::intrinsics::discriminant_value(other);
            __self_tag == __arg1_tag
        }
    }
    #[automatically_derived]
    impl ::core::cmp::Eq for FileSystemKind {
        #[inline]
        #[doc(hidden)]
        #[coverage(off)]
        fn assert_receiver_is_total_eq(&self) -> () {}
    }
    #[automatically_derived]
    impl ::core::marker::Copy for FileSystemKind {}
    #[automatically_derived]
    impl ::core::clone::Clone for FileSystemKind {
        #[inline]
        fn clone(&self) -> FileSystemKind {
            *self
        }
    }
    pub enum FileSystem {
        Stfs(StfsVolumeDescriptor),
        Svod(SvodVolumeDescriptor),
        Fatx,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for FileSystem {
        type Args<'__binrw_generated_args_lifetime> = (FileSystemKind,);
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let (mut fs_kind,) = __binrw_generated_var_arguments;
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                extern crate alloc;
                let mut __binrw_generated_error_basket: alloc::vec::Vec<
                    (&'static str, binrw::Error),
                > = alloc::vec::Vec::new();
                match (|| {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    binrw::__private::assert(
                        fs_kind == FileSystemKind::Stfs,
                        __binrw_generated_position_temp,
                        binrw::__private::AssertErrorFn::<
                            _,
                            fn() -> !,
                        >::Message(|| {
                            extern crate alloc;
                            {
                                let res = ::alloc::fmt::format(
                                    format_args!(
                                        "assertion failed: `{0}`",
                                        "fs_kind == FileSystemKind :: Stfs",
                                    ),
                                );
                                res
                            }
                        }),
                    )?;
                    let __binrw_generated_read_function = binrw::BinRead::read_options;
                    let mut self_0: StfsVolumeDescriptor = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'self_0' in FileSystem::Stfs"
                                    .into(),
                                line: 1045u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄─────╮\n   \u{1b}[1m1049\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mStfsVolumeDescriptor\u{1b}[0m\n  ┄─────╯\n",
                                ),
                            },
                        ))?;
                    let __binrw_this = Self::Stfs(self_0);
                    Ok(__binrw_this)
                })() {
                    ok @ Ok(_) => return ok,
                    Err(error) => {
                        binrw::__private::restore_position_variant(
                                __binrw_generated_var_reader,
                                __binrw_generated_position_temp,
                                error,
                            )
                            .map(|__binrw_temp| {
                                __binrw_generated_error_basket.push(("Stfs", __binrw_temp));
                            })?;
                    }
                }
                match (|| {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    binrw::__private::assert(
                        fs_kind == FileSystemKind::Svod,
                        __binrw_generated_position_temp,
                        binrw::__private::AssertErrorFn::<
                            _,
                            fn() -> !,
                        >::Message(|| {
                            extern crate alloc;
                            {
                                let res = ::alloc::fmt::format(
                                    format_args!(
                                        "assertion failed: `{0}`",
                                        "fs_kind == FileSystemKind :: Svod",
                                    ),
                                );
                                res
                            }
                        }),
                    )?;
                    let __binrw_generated_read_function = binrw::BinRead::read_options;
                    let mut self_0: SvodVolumeDescriptor = __binrw_generated_read_function(
                            __binrw_generated_var_reader,
                            __binrw_generated_var_endian,
                            <_ as binrw::__private::Required>::args(),
                        )
                        .map_err(|err| binrw::error::ContextExt::with_context(
                            err,
                            binrw::error::BacktraceFrame::Full {
                                message: "While parsing field 'self_0' in FileSystem::Svod"
                                    .into(),
                                line: 1045u32,
                                file: "stfs/src/parse.rs",
                                code: Some(
                                    "  ┄─────╮\n   \u{1b}[1m1051\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mSvodVolumeDescriptor\u{1b}[0m\n  ┄─────╯\n",
                                ),
                            },
                        ))?;
                    let __binrw_this = Self::Svod(self_0);
                    Ok(__binrw_this)
                })() {
                    ok @ Ok(_) => return ok,
                    Err(error) => {
                        binrw::__private::restore_position_variant(
                                __binrw_generated_var_reader,
                                __binrw_generated_position_temp,
                                error,
                            )
                            .map(|__binrw_temp| {
                                __binrw_generated_error_basket.push(("Svod", __binrw_temp));
                            })?;
                    }
                }
                match (|| {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    binrw::__private::assert(
                        fs_kind == FileSystemKind::Fatx,
                        __binrw_generated_position_temp,
                        binrw::__private::AssertErrorFn::<
                            _,
                            fn() -> !,
                        >::Message(|| {
                            extern crate alloc;
                            {
                                let res = ::alloc::fmt::format(
                                    format_args!(
                                        "assertion failed: `{0}`",
                                        "fs_kind == FileSystemKind :: Fatx",
                                    ),
                                );
                                res
                            }
                        }),
                    )?;
                    Ok(Self::Fatx)
                })() {
                    ok @ Ok(_) => return ok,
                    Err(error) => {
                        binrw::__private::restore_position_variant(
                                __binrw_generated_var_reader,
                                __binrw_generated_position_temp,
                                error,
                            )
                            .map(|__binrw_temp| {
                                __binrw_generated_error_basket.push(("Fatx", __binrw_temp));
                            })?;
                    }
                }
                Err(binrw::Error::EnumErrors {
                    pos: __binrw_generated_position_temp,
                    variant_errors: __binrw_generated_error_basket,
                })
            })()
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for FileSystem {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            match self {
                Self::Stfs(ref self_0) => {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                        binrw::BinWrite::write_options,
                    );
                    let __binrw_generated_args_self_0: <StfsVolumeDescriptor as binrw::BinWrite>::Args<
                        '_,
                    > = <_ as binrw::__private::Required>::args();
                    __binrw_generated_write_function(
                        &self_0,
                        __binrw_generated_var_writer,
                        __binrw_generated_var_endian,
                        __binrw_generated_args_self_0,
                    )?;
                }
                Self::Svod(ref self_0) => {
                    let __binrw_generated_var_endian = __binrw_generated_var_endian;
                    let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                        binrw::BinWrite::write_options,
                    );
                    let __binrw_generated_args_self_0: <SvodVolumeDescriptor as binrw::BinWrite>::Args<
                        '_,
                    > = <_ as binrw::__private::Required>::args();
                    __binrw_generated_write_function(
                        &self_0,
                        __binrw_generated_var_writer,
                        __binrw_generated_var_endian,
                        __binrw_generated_args_self_0,
                    )?;
                }
                Self::Fatx => {}
            }
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for FileSystem {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            match self {
                FileSystem::Stfs(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "Stfs",
                        &__self_0,
                    )
                }
                FileSystem::Svod(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "Svod",
                        &__self_0,
                    )
                }
                FileSystem::Fatx => ::core::fmt::Formatter::write_str(f, "Fatx"),
            }
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for FileSystem {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    FileSystem::Stfs(ref __field0) => {
                        _serde::Serializer::serialize_newtype_variant(
                            __serializer,
                            "FileSystem",
                            0u32,
                            "Stfs",
                            __field0,
                        )
                    }
                    FileSystem::Svod(ref __field0) => {
                        _serde::Serializer::serialize_newtype_variant(
                            __serializer,
                            "FileSystem",
                            1u32,
                            "Svod",
                            __field0,
                        )
                    }
                    FileSystem::Fatx => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "FileSystem",
                            2u32,
                            "Fatx",
                        )
                    }
                }
            }
        }
    };
    impl FileSystem {
        pub fn stfs(self) -> std::option::Option<((StfsVolumeDescriptor))> {
            match self {
                FileSystem::Stfs(ident_d667b4dd3c794d609d19cc12557cd5aa) => {
                    std::option::Option::Some(((ident_d667b4dd3c794d609d19cc12557cd5aa)))
                }
                _ => std::option::Option::None,
            }
        }
        pub fn stfs_ref(&self) -> std::option::Option<((&StfsVolumeDescriptor))> {
            match self {
                FileSystem::Stfs(ident_d667b4dd3c794d609d19cc12557cd5aa) => {
                    std::option::Option::Some(((ident_d667b4dd3c794d609d19cc12557cd5aa)))
                }
                _ => std::option::Option::None,
            }
        }
        pub fn stfs_mut(
            &mut self,
        ) -> std::option::Option<((&mut StfsVolumeDescriptor))> {
            match self {
                FileSystem::Stfs(ident_d667b4dd3c794d609d19cc12557cd5aa) => {
                    std::option::Option::Some(((ident_d667b4dd3c794d609d19cc12557cd5aa)))
                }
                _ => std::option::Option::None,
            }
        }
        pub fn stfs_or<E>(
            self,
            or: E,
        ) -> std::result::Result<((StfsVolumeDescriptor)), E> {
            self.stfs_or_else(|| or)
        }
        pub fn stfs_or_else<E, F: std::ops::FnOnce() -> E>(
            self,
            or_else: F,
        ) -> std::result::Result<((StfsVolumeDescriptor)), E> {
            match self {
                FileSystem::Stfs(ident_d667b4dd3c794d609d19cc12557cd5aa) => {
                    std::result::Result::Ok(((ident_d667b4dd3c794d609d19cc12557cd5aa)))
                }
                _ => std::result::Result::Err(or_else()),
            }
        }
        pub fn stfs_ref_or<E>(
            &self,
            or: E,
        ) -> std::result::Result<((&StfsVolumeDescriptor)), E> {
            self.stfs_ref_or_else(|| or)
        }
        pub fn stfs_mut_or<E>(
            &mut self,
            or: E,
        ) -> std::result::Result<((&mut StfsVolumeDescriptor)), E> {
            self.stfs_mut_or_else(|| or)
        }
        pub fn stfs_ref_or_else<E, F: std::ops::FnOnce() -> E>(
            &self,
            or_else: F,
        ) -> std::result::Result<((&StfsVolumeDescriptor)), E> {
            match self {
                FileSystem::Stfs(ident_d667b4dd3c794d609d19cc12557cd5aa) => {
                    std::result::Result::Ok(((ident_d667b4dd3c794d609d19cc12557cd5aa)))
                }
                _ => std::result::Result::Err(or_else()),
            }
        }
        pub fn stfs_mut_or_else<E, F: std::ops::FnOnce() -> E>(
            &mut self,
            or_else: F,
        ) -> std::result::Result<((&mut StfsVolumeDescriptor)), E> {
            match self {
                FileSystem::Stfs(ident_d667b4dd3c794d609d19cc12557cd5aa) => {
                    std::result::Result::Ok(((ident_d667b4dd3c794d609d19cc12557cd5aa)))
                }
                _ => std::result::Result::Err(or_else()),
            }
        }
        pub fn and_then_stfs<
            F: std::ops::FnOnce(((StfsVolumeDescriptor))) -> ((StfsVolumeDescriptor)),
        >(self, and_then: F) -> Self {
            match self {
                FileSystem::Stfs(ident_d667b4dd3c794d609d19cc12557cd5aa) => {
                    let (ident_d667b4dd3c794d609d19cc12557cd5aa) = and_then(
                        (ident_d667b4dd3c794d609d19cc12557cd5aa),
                    );
                    FileSystem::Stfs(ident_d667b4dd3c794d609d19cc12557cd5aa)
                }
                _ => self,
            }
        }
        pub fn expect_stfs(self, msg: &str) -> ((StfsVolumeDescriptor)) {
            self.unwrap_or_else_stfs(|| {
                ::std::rt::panic_display(&msg);
            })
        }
        #[deprecated(
            since = "0.2.0",
            note = "Please use the derived `FileSystem::stfs` method instead. This method will be removed in 1.0.0 or next pre-stable minor bump."
        )]
        pub fn ok_stfs(self) -> std::option::Option<((StfsVolumeDescriptor))> {
            self.stfs()
        }
        #[deprecated(
            since = "0.2.0",
            note = "Please use the derived `FileSystem::stfs_or` method instead. This method will be removed in 1.0.0 or next pre-stable minor bump."
        )]
        pub fn ok_or_stfs<E>(
            self,
            or: E,
        ) -> std::result::Result<((StfsVolumeDescriptor)), E> {
            self.stfs_or(or)
        }
        #[deprecated(
            since = "0.2.0",
            note = "Please use the derived `FileSystem::stfs_or_else` method instead. This method will be removed in 1.0.0 or next pre-stable minor bump."
        )]
        pub fn ok_or_else_stfs<E, F: std::ops::FnOnce() -> E>(
            self,
            or_else: F,
        ) -> std::result::Result<((StfsVolumeDescriptor)), E> {
            self.stfs_or_else(or_else)
        }
        pub fn or_else_stfs<F: std::ops::FnOnce() -> ((StfsVolumeDescriptor))>(
            self,
            or_else: F,
        ) -> Self {
            match self {
                FileSystem::Stfs(ident_d667b4dd3c794d609d19cc12557cd5aa) => {
                    FileSystem::Stfs(ident_d667b4dd3c794d609d19cc12557cd5aa)
                }
                _ => {
                    let (ident_d667b4dd3c794d609d19cc12557cd5aa) = or_else();
                    FileSystem::Stfs(ident_d667b4dd3c794d609d19cc12557cd5aa)
                }
            }
        }
        pub fn unwrap_stfs(self) -> ((StfsVolumeDescriptor)) {
            self.unwrap_or_else_stfs(|| { ::std::rt::begin_panic("explicit panic") })
        }
        pub fn unwrap_or_stfs(
            self,
            or: ((StfsVolumeDescriptor)),
        ) -> ((StfsVolumeDescriptor)) {
            self.unwrap_or_else_stfs(|| or)
        }
        pub fn unwrap_or_else_stfs<F: std::ops::FnOnce() -> ((StfsVolumeDescriptor))>(
            self,
            or_else: F,
        ) -> ((StfsVolumeDescriptor)) {
            match self {
                FileSystem::Stfs(ident_d667b4dd3c794d609d19cc12557cd5aa) => {
                    ((ident_d667b4dd3c794d609d19cc12557cd5aa))
                }
                _ => or_else(),
            }
        }
        pub fn is_stfs(&self) -> bool {
            match self {
                FileSystem::Stfs(..) => true,
                _ => false,
            }
        }
        pub fn is_not_stfs(&self) -> bool {
            !self.is_stfs()
        }
        pub fn and_stfs(self, and: Self) -> Self {
            match (&self, &and) {
                (FileSystem::Stfs(..), FileSystem::Stfs(..)) => and,
                _ => self,
            }
        }
        pub fn or_stfs(self, or: Self) -> Self {
            match &self {
                FileSystem::Stfs(..) => self,
                _ => or,
            }
        }
        pub fn svod(self) -> std::option::Option<((SvodVolumeDescriptor))> {
            match self {
                FileSystem::Svod(ident_6b1ef3a7513b43fea48d883750cad29c) => {
                    std::option::Option::Some(((ident_6b1ef3a7513b43fea48d883750cad29c)))
                }
                _ => std::option::Option::None,
            }
        }
        pub fn svod_ref(&self) -> std::option::Option<((&SvodVolumeDescriptor))> {
            match self {
                FileSystem::Svod(ident_6b1ef3a7513b43fea48d883750cad29c) => {
                    std::option::Option::Some(((ident_6b1ef3a7513b43fea48d883750cad29c)))
                }
                _ => std::option::Option::None,
            }
        }
        pub fn svod_mut(
            &mut self,
        ) -> std::option::Option<((&mut SvodVolumeDescriptor))> {
            match self {
                FileSystem::Svod(ident_6b1ef3a7513b43fea48d883750cad29c) => {
                    std::option::Option::Some(((ident_6b1ef3a7513b43fea48d883750cad29c)))
                }
                _ => std::option::Option::None,
            }
        }
        pub fn svod_or<E>(
            self,
            or: E,
        ) -> std::result::Result<((SvodVolumeDescriptor)), E> {
            self.svod_or_else(|| or)
        }
        pub fn svod_or_else<E, F: std::ops::FnOnce() -> E>(
            self,
            or_else: F,
        ) -> std::result::Result<((SvodVolumeDescriptor)), E> {
            match self {
                FileSystem::Svod(ident_6b1ef3a7513b43fea48d883750cad29c) => {
                    std::result::Result::Ok(((ident_6b1ef3a7513b43fea48d883750cad29c)))
                }
                _ => std::result::Result::Err(or_else()),
            }
        }
        pub fn svod_ref_or<E>(
            &self,
            or: E,
        ) -> std::result::Result<((&SvodVolumeDescriptor)), E> {
            self.svod_ref_or_else(|| or)
        }
        pub fn svod_mut_or<E>(
            &mut self,
            or: E,
        ) -> std::result::Result<((&mut SvodVolumeDescriptor)), E> {
            self.svod_mut_or_else(|| or)
        }
        pub fn svod_ref_or_else<E, F: std::ops::FnOnce() -> E>(
            &self,
            or_else: F,
        ) -> std::result::Result<((&SvodVolumeDescriptor)), E> {
            match self {
                FileSystem::Svod(ident_6b1ef3a7513b43fea48d883750cad29c) => {
                    std::result::Result::Ok(((ident_6b1ef3a7513b43fea48d883750cad29c)))
                }
                _ => std::result::Result::Err(or_else()),
            }
        }
        pub fn svod_mut_or_else<E, F: std::ops::FnOnce() -> E>(
            &mut self,
            or_else: F,
        ) -> std::result::Result<((&mut SvodVolumeDescriptor)), E> {
            match self {
                FileSystem::Svod(ident_6b1ef3a7513b43fea48d883750cad29c) => {
                    std::result::Result::Ok(((ident_6b1ef3a7513b43fea48d883750cad29c)))
                }
                _ => std::result::Result::Err(or_else()),
            }
        }
        pub fn and_then_svod<
            F: std::ops::FnOnce(((SvodVolumeDescriptor))) -> ((SvodVolumeDescriptor)),
        >(self, and_then: F) -> Self {
            match self {
                FileSystem::Svod(ident_6b1ef3a7513b43fea48d883750cad29c) => {
                    let (ident_6b1ef3a7513b43fea48d883750cad29c) = and_then(
                        (ident_6b1ef3a7513b43fea48d883750cad29c),
                    );
                    FileSystem::Svod(ident_6b1ef3a7513b43fea48d883750cad29c)
                }
                _ => self,
            }
        }
        pub fn expect_svod(self, msg: &str) -> ((SvodVolumeDescriptor)) {
            self.unwrap_or_else_svod(|| {
                ::std::rt::panic_display(&msg);
            })
        }
        #[deprecated(
            since = "0.2.0",
            note = "Please use the derived `FileSystem::svod` method instead. This method will be removed in 1.0.0 or next pre-stable minor bump."
        )]
        pub fn ok_svod(self) -> std::option::Option<((SvodVolumeDescriptor))> {
            self.svod()
        }
        #[deprecated(
            since = "0.2.0",
            note = "Please use the derived `FileSystem::svod_or` method instead. This method will be removed in 1.0.0 or next pre-stable minor bump."
        )]
        pub fn ok_or_svod<E>(
            self,
            or: E,
        ) -> std::result::Result<((SvodVolumeDescriptor)), E> {
            self.svod_or(or)
        }
        #[deprecated(
            since = "0.2.0",
            note = "Please use the derived `FileSystem::svod_or_else` method instead. This method will be removed in 1.0.0 or next pre-stable minor bump."
        )]
        pub fn ok_or_else_svod<E, F: std::ops::FnOnce() -> E>(
            self,
            or_else: F,
        ) -> std::result::Result<((SvodVolumeDescriptor)), E> {
            self.svod_or_else(or_else)
        }
        pub fn or_else_svod<F: std::ops::FnOnce() -> ((SvodVolumeDescriptor))>(
            self,
            or_else: F,
        ) -> Self {
            match self {
                FileSystem::Svod(ident_6b1ef3a7513b43fea48d883750cad29c) => {
                    FileSystem::Svod(ident_6b1ef3a7513b43fea48d883750cad29c)
                }
                _ => {
                    let (ident_6b1ef3a7513b43fea48d883750cad29c) = or_else();
                    FileSystem::Svod(ident_6b1ef3a7513b43fea48d883750cad29c)
                }
            }
        }
        pub fn unwrap_svod(self) -> ((SvodVolumeDescriptor)) {
            self.unwrap_or_else_svod(|| { ::std::rt::begin_panic("explicit panic") })
        }
        pub fn unwrap_or_svod(
            self,
            or: ((SvodVolumeDescriptor)),
        ) -> ((SvodVolumeDescriptor)) {
            self.unwrap_or_else_svod(|| or)
        }
        pub fn unwrap_or_else_svod<F: std::ops::FnOnce() -> ((SvodVolumeDescriptor))>(
            self,
            or_else: F,
        ) -> ((SvodVolumeDescriptor)) {
            match self {
                FileSystem::Svod(ident_6b1ef3a7513b43fea48d883750cad29c) => {
                    ((ident_6b1ef3a7513b43fea48d883750cad29c))
                }
                _ => or_else(),
            }
        }
        pub fn is_svod(&self) -> bool {
            match self {
                FileSystem::Svod(..) => true,
                _ => false,
            }
        }
        pub fn is_not_svod(&self) -> bool {
            !self.is_svod()
        }
        pub fn and_svod(self, and: Self) -> Self {
            match (&self, &and) {
                (FileSystem::Svod(..), FileSystem::Svod(..)) => and,
                _ => self,
            }
        }
        pub fn or_svod(self, or: Self) -> Self {
            match &self {
                FileSystem::Svod(..) => self,
                _ => or,
            }
        }
        pub fn is_fatx(&self) -> bool {
            match self {
                FileSystem::Fatx => true,
                _ => false,
            }
        }
        pub fn is_not_fatx(&self) -> bool {
            !self.is_fatx()
        }
        pub fn and_fatx(self, and: Self) -> Self {
            match (&self, &and) {
                (FileSystem::Fatx, FileSystem::Fatx) => and,
                _ => self,
            }
        }
        pub fn or_fatx(self, or: Self) -> Self {
            match &self {
                FileSystem::Fatx => self,
                _ => or,
            }
        }
    }
    impl Default for FileSystem {
        fn default() -> Self {
            FileSystem::Stfs(StfsVolumeDescriptor::default())
        }
    }
    impl FileSystem {}
    #[allow(clippy::identity_op)]
    pub struct StfsVolumeDescriptorFlags {
        bytes: [::core::primitive::u8; {
            ((({
                0usize + <B4 as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS
            } - 1) / 8) + 1) * 8
        } / 8usize],
    }
    #[automatically_derived]
    #[allow(clippy::identity_op)]
    impl ::core::default::Default for StfsVolumeDescriptorFlags {
        #[inline]
        fn default() -> StfsVolumeDescriptorFlags {
            StfsVolumeDescriptorFlags {
                bytes: ::core::default::Default::default(),
            }
        }
    }
    #[automatically_derived]
    #[allow(clippy::identity_op)]
    impl ::core::marker::Copy for StfsVolumeDescriptorFlags {}
    #[automatically_derived]
    #[allow(clippy::identity_op)]
    impl ::core::clone::Clone for StfsVolumeDescriptorFlags {
        #[inline]
        fn clone(&self) -> StfsVolumeDescriptorFlags {
            let _: ::core::clone::AssertParamIsClone<
                [::core::primitive::u8; {
                    ((({
                        0usize + <B4 as ::modular_bitfield::Specifier>::BITS
                            + <bool as ::modular_bitfield::Specifier>::BITS
                            + <bool as ::modular_bitfield::Specifier>::BITS
                            + <bool as ::modular_bitfield::Specifier>::BITS
                            + <bool as ::modular_bitfield::Specifier>::BITS
                    } - 1) / 8) + 1) * 8
                } / 8usize],
            >;
            *self
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for StfsVolumeDescriptorFlags {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "StfsVolumeDescriptorFlags",
                    false as usize + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "bytes",
                    &self.bytes,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for StfsVolumeDescriptorFlags {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                binrw::BinRead::read_options(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        (),
                    )
                    .map(Self::from_bytes)
                    .and_then(|__binrw_this| {
                        let Self { ref bytes } = &__binrw_this;
                        (|| { Ok(()) })().map(|_: ()| __binrw_this)
                    })
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    impl binrw::meta::ReadEndian for StfsVolumeDescriptorFlags {
        const ENDIAN: binrw::meta::EndianKind = binrw::meta::EndianKind::None;
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for StfsVolumeDescriptorFlags {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            binrw::BinWrite::write_options(
                &((|flags: &Self| flags.into_bytes())(self)),
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                (),
            )?;
            Ok(())
        }
    }
    impl binrw::meta::WriteEndian for StfsVolumeDescriptorFlags {
        const ENDIAN: binrw::meta::EndianKind = binrw::meta::EndianKind::None;
    }
    #[allow(clippy::identity_op)]
    const _: () = {
        impl ::modular_bitfield::private::checks::CheckTotalSizeMultipleOf8
        for StfsVolumeDescriptorFlags {
            type Size = ::modular_bitfield::private::checks::TotalSize<
                [(); {
                    0usize + <B4 as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS
                } % 8usize],
            >;
        }
    };
    impl StfsVolumeDescriptorFlags {
        /// Returns an instance with zero initialized data.
        #[allow(clippy::identity_op)]
        pub const fn new() -> Self {
            Self {
                bytes: [0u8; {
                    ((({
                        0usize + <B4 as ::modular_bitfield::Specifier>::BITS
                            + <bool as ::modular_bitfield::Specifier>::BITS
                            + <bool as ::modular_bitfield::Specifier>::BITS
                            + <bool as ::modular_bitfield::Specifier>::BITS
                            + <bool as ::modular_bitfield::Specifier>::BITS
                    } - 1) / 8) + 1) * 8
                } / 8usize],
            }
        }
    }
    impl StfsVolumeDescriptorFlags {
        /// Returns the underlying bits.
        ///
        /// # Layout
        ///
        /// The returned byte array is layed out in the same way as described
        /// [here](https://docs.rs/modular-bitfield/#generated-structure).
        #[inline]
        #[allow(clippy::identity_op)]
        pub const fn into_bytes(
            self,
        ) -> [::core::primitive::u8; {
            ((({
                0usize + <B4 as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS
            } - 1) / 8) + 1) * 8
        } / 8usize] {
            self.bytes
        }
        /// Converts the given bytes directly into the bitfield struct.
        #[inline]
        #[allow(clippy::identity_op)]
        pub const fn from_bytes(
            bytes: [::core::primitive::u8; {
                ((({
                    0usize + <B4 as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS
                } - 1) / 8) + 1) * 8
            } / 8usize],
        ) -> Self {
            Self { bytes }
        }
    }
    const _: () = {
        const _: () = {};
        const _: () = {};
        const _: () = {};
        const _: () = {};
        const _: () = {};
    };
    impl StfsVolumeDescriptorFlags {
        ///Returns the value of _reserved.
        #[inline]
        fn _reserved(&self) -> <B4 as ::modular_bitfield::Specifier>::InOut {
            self._reserved_or_err()
                .expect(
                    "value contains invalid bit pattern for field StfsVolumeDescriptorFlags._reserved",
                )
        }
        /**Returns the value of _reserved.

#Errors

If the returned value contains an invalid bit pattern for _reserved.*/
        #[inline]
        #[allow(dead_code)]
        fn _reserved_or_err(
            &self,
        ) -> ::core::result::Result<
            <B4 as ::modular_bitfield::Specifier>::InOut,
            ::modular_bitfield::error::InvalidBitPattern<
                <B4 as ::modular_bitfield::Specifier>::Bytes,
            >,
        > {
            let __bf_read: <B4 as ::modular_bitfield::Specifier>::Bytes = {
                ::modular_bitfield::private::read_specifier::<
                    B4,
                >(&self.bytes[..], 0usize)
            };
            <B4 as ::modular_bitfield::Specifier>::from_bytes(__bf_read)
        }
        /**Returns a copy of the bitfield with the value of _reserved set to the given value.

#Panics

If the given value is out of bounds for _reserved.*/
        #[inline]
        #[allow(dead_code)]
        fn with__reserved(
            mut self,
            new_val: <B4 as ::modular_bitfield::Specifier>::InOut,
        ) -> Self {
            self.set__reserved(new_val);
            self
        }
        /**Returns a copy of the bitfield with the value of _reserved set to the given value.

#Errors

If the given value is out of bounds for _reserved.*/
        #[inline]
        #[allow(dead_code)]
        fn with__reserved_checked(
            mut self,
            new_val: <B4 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<Self, ::modular_bitfield::error::OutOfBounds> {
            self.set__reserved_checked(new_val)?;
            ::core::result::Result::Ok(self)
        }
        /**Sets the value of _reserved to the given value.

#Panics

If the given value is out of bounds for _reserved.*/
        #[inline]
        #[allow(dead_code)]
        fn set__reserved(
            &mut self,
            new_val: <B4 as ::modular_bitfield::Specifier>::InOut,
        ) {
            self.set__reserved_checked(new_val)
                .expect(
                    "value out of bounds for field StfsVolumeDescriptorFlags._reserved",
                )
        }
        /**Sets the value of _reserved to the given value.

#Errors

If the given value is out of bounds for _reserved.*/
        #[inline]
        fn set__reserved_checked(
            &mut self,
            new_val: <B4 as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<(), ::modular_bitfield::error::OutOfBounds> {
            let __bf_base_bits: ::core::primitive::usize = 8usize
                * ::core::mem::size_of::<<B4 as ::modular_bitfield::Specifier>::Bytes>();
            let __bf_max_value: <B4 as ::modular_bitfield::Specifier>::Bytes = {
                !0 >> (__bf_base_bits - <B4 as ::modular_bitfield::Specifier>::BITS)
            };
            let __bf_spec_bits: ::core::primitive::usize = <B4 as ::modular_bitfield::Specifier>::BITS;
            let __bf_raw_val: <B4 as ::modular_bitfield::Specifier>::Bytes = {
                <B4 as ::modular_bitfield::Specifier>::into_bytes(new_val)
            }?;
            if !(__bf_base_bits == __bf_spec_bits || __bf_raw_val <= __bf_max_value) {
                return ::core::result::Result::Err(
                    ::modular_bitfield::error::OutOfBounds,
                );
            }
            ::modular_bitfield::private::write_specifier::<
                B4,
            >(&mut self.bytes[..], 0usize, __bf_raw_val);
            ::core::result::Result::Ok(())
        }
        ///Returns the value of dir_index_bounds_are_valid.
        #[inline]
        fn dir_index_bounds_are_valid(
            &self,
        ) -> <bool as ::modular_bitfield::Specifier>::InOut {
            self.dir_index_bounds_are_valid_or_err()
                .expect(
                    "value contains invalid bit pattern for field StfsVolumeDescriptorFlags.dir_index_bounds_are_valid",
                )
        }
        /**Returns the value of dir_index_bounds_are_valid.

#Errors

If the returned value contains an invalid bit pattern for dir_index_bounds_are_valid.*/
        #[inline]
        #[allow(dead_code)]
        fn dir_index_bounds_are_valid_or_err(
            &self,
        ) -> ::core::result::Result<
            <bool as ::modular_bitfield::Specifier>::InOut,
            ::modular_bitfield::error::InvalidBitPattern<
                <bool as ::modular_bitfield::Specifier>::Bytes,
            >,
        > {
            let __bf_read: <bool as ::modular_bitfield::Specifier>::Bytes = {
                ::modular_bitfield::private::read_specifier::<
                    bool,
                >(&self.bytes[..], 0usize + <B4 as ::modular_bitfield::Specifier>::BITS)
            };
            <bool as ::modular_bitfield::Specifier>::from_bytes(__bf_read)
        }
        /**Returns a copy of the bitfield with the value of dir_index_bounds_are_valid set to the given value.

#Panics

If the given value is out of bounds for dir_index_bounds_are_valid.*/
        #[inline]
        #[allow(dead_code)]
        fn with_dir_index_bounds_are_valid(
            mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> Self {
            self.set_dir_index_bounds_are_valid(new_val);
            self
        }
        /**Returns a copy of the bitfield with the value of dir_index_bounds_are_valid set to the given value.

#Errors

If the given value is out of bounds for dir_index_bounds_are_valid.*/
        #[inline]
        #[allow(dead_code)]
        fn with_dir_index_bounds_are_valid_checked(
            mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<Self, ::modular_bitfield::error::OutOfBounds> {
            self.set_dir_index_bounds_are_valid_checked(new_val)?;
            ::core::result::Result::Ok(self)
        }
        /**Sets the value of dir_index_bounds_are_valid to the given value.

#Panics

If the given value is out of bounds for dir_index_bounds_are_valid.*/
        #[inline]
        #[allow(dead_code)]
        fn set_dir_index_bounds_are_valid(
            &mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) {
            self.set_dir_index_bounds_are_valid_checked(new_val)
                .expect(
                    "value out of bounds for field StfsVolumeDescriptorFlags.dir_index_bounds_are_valid",
                )
        }
        /**Sets the value of dir_index_bounds_are_valid to the given value.

#Errors

If the given value is out of bounds for dir_index_bounds_are_valid.*/
        #[inline]
        fn set_dir_index_bounds_are_valid_checked(
            &mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<(), ::modular_bitfield::error::OutOfBounds> {
            let __bf_base_bits: ::core::primitive::usize = 8usize
                * ::core::mem::size_of::<
                    <bool as ::modular_bitfield::Specifier>::Bytes,
                >();
            let __bf_max_value: <bool as ::modular_bitfield::Specifier>::Bytes = {
                !0 >> (__bf_base_bits - <bool as ::modular_bitfield::Specifier>::BITS)
            };
            let __bf_spec_bits: ::core::primitive::usize = <bool as ::modular_bitfield::Specifier>::BITS;
            let __bf_raw_val: <bool as ::modular_bitfield::Specifier>::Bytes = {
                <bool as ::modular_bitfield::Specifier>::into_bytes(new_val)
            }?;
            if !(__bf_base_bits == __bf_spec_bits || __bf_raw_val <= __bf_max_value) {
                return ::core::result::Result::Err(
                    ::modular_bitfield::error::OutOfBounds,
                );
            }
            ::modular_bitfield::private::write_specifier::<
                bool,
            >(
                &mut self.bytes[..],
                0usize + <B4 as ::modular_bitfield::Specifier>::BITS,
                __bf_raw_val,
            );
            ::core::result::Result::Ok(())
        }
        ///Returns the value of dir_is_overallocated.
        #[inline]
        fn dir_is_overallocated(
            &self,
        ) -> <bool as ::modular_bitfield::Specifier>::InOut {
            self.dir_is_overallocated_or_err()
                .expect(
                    "value contains invalid bit pattern for field StfsVolumeDescriptorFlags.dir_is_overallocated",
                )
        }
        /**Returns the value of dir_is_overallocated.

#Errors

If the returned value contains an invalid bit pattern for dir_is_overallocated.*/
        #[inline]
        #[allow(dead_code)]
        fn dir_is_overallocated_or_err(
            &self,
        ) -> ::core::result::Result<
            <bool as ::modular_bitfield::Specifier>::InOut,
            ::modular_bitfield::error::InvalidBitPattern<
                <bool as ::modular_bitfield::Specifier>::Bytes,
            >,
        > {
            let __bf_read: <bool as ::modular_bitfield::Specifier>::Bytes = {
                ::modular_bitfield::private::read_specifier::<
                    bool,
                >(
                    &self.bytes[..],
                    0usize + <B4 as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS,
                )
            };
            <bool as ::modular_bitfield::Specifier>::from_bytes(__bf_read)
        }
        /**Returns a copy of the bitfield with the value of dir_is_overallocated set to the given value.

#Panics

If the given value is out of bounds for dir_is_overallocated.*/
        #[inline]
        #[allow(dead_code)]
        fn with_dir_is_overallocated(
            mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> Self {
            self.set_dir_is_overallocated(new_val);
            self
        }
        /**Returns a copy of the bitfield with the value of dir_is_overallocated set to the given value.

#Errors

If the given value is out of bounds for dir_is_overallocated.*/
        #[inline]
        #[allow(dead_code)]
        fn with_dir_is_overallocated_checked(
            mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<Self, ::modular_bitfield::error::OutOfBounds> {
            self.set_dir_is_overallocated_checked(new_val)?;
            ::core::result::Result::Ok(self)
        }
        /**Sets the value of dir_is_overallocated to the given value.

#Panics

If the given value is out of bounds for dir_is_overallocated.*/
        #[inline]
        #[allow(dead_code)]
        fn set_dir_is_overallocated(
            &mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) {
            self.set_dir_is_overallocated_checked(new_val)
                .expect(
                    "value out of bounds for field StfsVolumeDescriptorFlags.dir_is_overallocated",
                )
        }
        /**Sets the value of dir_is_overallocated to the given value.

#Errors

If the given value is out of bounds for dir_is_overallocated.*/
        #[inline]
        fn set_dir_is_overallocated_checked(
            &mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<(), ::modular_bitfield::error::OutOfBounds> {
            let __bf_base_bits: ::core::primitive::usize = 8usize
                * ::core::mem::size_of::<
                    <bool as ::modular_bitfield::Specifier>::Bytes,
                >();
            let __bf_max_value: <bool as ::modular_bitfield::Specifier>::Bytes = {
                !0 >> (__bf_base_bits - <bool as ::modular_bitfield::Specifier>::BITS)
            };
            let __bf_spec_bits: ::core::primitive::usize = <bool as ::modular_bitfield::Specifier>::BITS;
            let __bf_raw_val: <bool as ::modular_bitfield::Specifier>::Bytes = {
                <bool as ::modular_bitfield::Specifier>::into_bytes(new_val)
            }?;
            if !(__bf_base_bits == __bf_spec_bits || __bf_raw_val <= __bf_max_value) {
                return ::core::result::Result::Err(
                    ::modular_bitfield::error::OutOfBounds,
                );
            }
            ::modular_bitfield::private::write_specifier::<
                bool,
            >(
                &mut self.bytes[..],
                0usize + <B4 as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS,
                __bf_raw_val,
            );
            ::core::result::Result::Ok(())
        }
        ///Returns the value of root_active_index.
        #[inline]
        fn root_active_index(&self) -> <bool as ::modular_bitfield::Specifier>::InOut {
            self.root_active_index_or_err()
                .expect(
                    "value contains invalid bit pattern for field StfsVolumeDescriptorFlags.root_active_index",
                )
        }
        /**Returns the value of root_active_index.

#Errors

If the returned value contains an invalid bit pattern for root_active_index.*/
        #[inline]
        #[allow(dead_code)]
        fn root_active_index_or_err(
            &self,
        ) -> ::core::result::Result<
            <bool as ::modular_bitfield::Specifier>::InOut,
            ::modular_bitfield::error::InvalidBitPattern<
                <bool as ::modular_bitfield::Specifier>::Bytes,
            >,
        > {
            let __bf_read: <bool as ::modular_bitfield::Specifier>::Bytes = {
                ::modular_bitfield::private::read_specifier::<
                    bool,
                >(
                    &self.bytes[..],
                    0usize + <B4 as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS,
                )
            };
            <bool as ::modular_bitfield::Specifier>::from_bytes(__bf_read)
        }
        /**Returns a copy of the bitfield with the value of root_active_index set to the given value.

#Panics

If the given value is out of bounds for root_active_index.*/
        #[inline]
        #[allow(dead_code)]
        fn with_root_active_index(
            mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> Self {
            self.set_root_active_index(new_val);
            self
        }
        /**Returns a copy of the bitfield with the value of root_active_index set to the given value.

#Errors

If the given value is out of bounds for root_active_index.*/
        #[inline]
        #[allow(dead_code)]
        fn with_root_active_index_checked(
            mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<Self, ::modular_bitfield::error::OutOfBounds> {
            self.set_root_active_index_checked(new_val)?;
            ::core::result::Result::Ok(self)
        }
        /**Sets the value of root_active_index to the given value.

#Panics

If the given value is out of bounds for root_active_index.*/
        #[inline]
        #[allow(dead_code)]
        fn set_root_active_index(
            &mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) {
            self.set_root_active_index_checked(new_val)
                .expect(
                    "value out of bounds for field StfsVolumeDescriptorFlags.root_active_index",
                )
        }
        /**Sets the value of root_active_index to the given value.

#Errors

If the given value is out of bounds for root_active_index.*/
        #[inline]
        fn set_root_active_index_checked(
            &mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<(), ::modular_bitfield::error::OutOfBounds> {
            let __bf_base_bits: ::core::primitive::usize = 8usize
                * ::core::mem::size_of::<
                    <bool as ::modular_bitfield::Specifier>::Bytes,
                >();
            let __bf_max_value: <bool as ::modular_bitfield::Specifier>::Bytes = {
                !0 >> (__bf_base_bits - <bool as ::modular_bitfield::Specifier>::BITS)
            };
            let __bf_spec_bits: ::core::primitive::usize = <bool as ::modular_bitfield::Specifier>::BITS;
            let __bf_raw_val: <bool as ::modular_bitfield::Specifier>::Bytes = {
                <bool as ::modular_bitfield::Specifier>::into_bytes(new_val)
            }?;
            if !(__bf_base_bits == __bf_spec_bits || __bf_raw_val <= __bf_max_value) {
                return ::core::result::Result::Err(
                    ::modular_bitfield::error::OutOfBounds,
                );
            }
            ::modular_bitfield::private::write_specifier::<
                bool,
            >(
                &mut self.bytes[..],
                0usize + <B4 as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS,
                __bf_raw_val,
            );
            ::core::result::Result::Ok(())
        }
        ///Returns the value of read_only.
        #[inline]
        fn read_only(&self) -> <bool as ::modular_bitfield::Specifier>::InOut {
            self.read_only_or_err()
                .expect(
                    "value contains invalid bit pattern for field StfsVolumeDescriptorFlags.read_only",
                )
        }
        /**Returns the value of read_only.

#Errors

If the returned value contains an invalid bit pattern for read_only.*/
        #[inline]
        #[allow(dead_code)]
        fn read_only_or_err(
            &self,
        ) -> ::core::result::Result<
            <bool as ::modular_bitfield::Specifier>::InOut,
            ::modular_bitfield::error::InvalidBitPattern<
                <bool as ::modular_bitfield::Specifier>::Bytes,
            >,
        > {
            let __bf_read: <bool as ::modular_bitfield::Specifier>::Bytes = {
                ::modular_bitfield::private::read_specifier::<
                    bool,
                >(
                    &self.bytes[..],
                    0usize + <B4 as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS
                        + <bool as ::modular_bitfield::Specifier>::BITS,
                )
            };
            <bool as ::modular_bitfield::Specifier>::from_bytes(__bf_read)
        }
        /**Returns a copy of the bitfield with the value of read_only set to the given value.

#Panics

If the given value is out of bounds for read_only.*/
        #[inline]
        #[allow(dead_code)]
        fn with_read_only(
            mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> Self {
            self.set_read_only(new_val);
            self
        }
        /**Returns a copy of the bitfield with the value of read_only set to the given value.

#Errors

If the given value is out of bounds for read_only.*/
        #[inline]
        #[allow(dead_code)]
        fn with_read_only_checked(
            mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<Self, ::modular_bitfield::error::OutOfBounds> {
            self.set_read_only_checked(new_val)?;
            ::core::result::Result::Ok(self)
        }
        /**Sets the value of read_only to the given value.

#Panics

If the given value is out of bounds for read_only.*/
        #[inline]
        #[allow(dead_code)]
        fn set_read_only(
            &mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) {
            self.set_read_only_checked(new_val)
                .expect(
                    "value out of bounds for field StfsVolumeDescriptorFlags.read_only",
                )
        }
        /**Sets the value of read_only to the given value.

#Errors

If the given value is out of bounds for read_only.*/
        #[inline]
        fn set_read_only_checked(
            &mut self,
            new_val: <bool as ::modular_bitfield::Specifier>::InOut,
        ) -> ::core::result::Result<(), ::modular_bitfield::error::OutOfBounds> {
            let __bf_base_bits: ::core::primitive::usize = 8usize
                * ::core::mem::size_of::<
                    <bool as ::modular_bitfield::Specifier>::Bytes,
                >();
            let __bf_max_value: <bool as ::modular_bitfield::Specifier>::Bytes = {
                !0 >> (__bf_base_bits - <bool as ::modular_bitfield::Specifier>::BITS)
            };
            let __bf_spec_bits: ::core::primitive::usize = <bool as ::modular_bitfield::Specifier>::BITS;
            let __bf_raw_val: <bool as ::modular_bitfield::Specifier>::Bytes = {
                <bool as ::modular_bitfield::Specifier>::into_bytes(new_val)
            }?;
            if !(__bf_base_bits == __bf_spec_bits || __bf_raw_val <= __bf_max_value) {
                return ::core::result::Result::Err(
                    ::modular_bitfield::error::OutOfBounds,
                );
            }
            ::modular_bitfield::private::write_specifier::<
                bool,
            >(
                &mut self.bytes[..],
                0usize + <B4 as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS
                    + <bool as ::modular_bitfield::Specifier>::BITS,
                __bf_raw_val,
            );
            ::core::result::Result::Ok(())
        }
    }
    impl ::core::fmt::Debug for StfsVolumeDescriptorFlags {
        fn fmt(&self, __bf_f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
            __bf_f
                .debug_struct("StfsVolumeDescriptorFlags")
                .field(
                    "_reserved",
                    self
                        ._reserved_or_err()
                        .as_ref()
                        .map(|__bf_field| __bf_field as &dyn ::core::fmt::Debug)
                        .unwrap_or_else(|__bf_err| __bf_err as &dyn ::core::fmt::Debug),
                )
                .field(
                    "dir_index_bounds_are_valid",
                    self
                        .dir_index_bounds_are_valid_or_err()
                        .as_ref()
                        .map(|__bf_field| __bf_field as &dyn ::core::fmt::Debug)
                        .unwrap_or_else(|__bf_err| __bf_err as &dyn ::core::fmt::Debug),
                )
                .field(
                    "dir_is_overallocated",
                    self
                        .dir_is_overallocated_or_err()
                        .as_ref()
                        .map(|__bf_field| __bf_field as &dyn ::core::fmt::Debug)
                        .unwrap_or_else(|__bf_err| __bf_err as &dyn ::core::fmt::Debug),
                )
                .field(
                    "root_active_index",
                    self
                        .root_active_index_or_err()
                        .as_ref()
                        .map(|__bf_field| __bf_field as &dyn ::core::fmt::Debug)
                        .unwrap_or_else(|__bf_err| __bf_err as &dyn ::core::fmt::Debug),
                )
                .field(
                    "read_only",
                    self
                        .read_only_or_err()
                        .as_ref()
                        .map(|__bf_field| __bf_field as &dyn ::core::fmt::Debug)
                        .unwrap_or_else(|__bf_err| __bf_err as &dyn ::core::fmt::Debug),
                )
                .finish()
        }
    }
    pub struct StfsVolumeDescriptor {
        size: u8,
        version: u8,
        flags: StfsVolumeDescriptorFlags,
        file_table_block_count: u16,
        file_table_block_num: Block,
        top_hash_table_hash: [u8; 0x14],
        allocated_block_count: u32,
        unallocated_block_count: u32,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for StfsVolumeDescriptor {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut size: u8 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'size' in StfsVolumeDescriptor"
                                .into(),
                            line: 1080u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   \u{1b}[1m1080\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1msize: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut version: u8 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'version' in StfsVolumeDescriptor"
                                .into(),
                            line: 1081u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   \u{1b}[1m1081\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mversion: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut flags: StfsVolumeDescriptorFlags = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'flags' in StfsVolumeDescriptor"
                                .into(),
                            line: 1082u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   \u{1b}[1m1082\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mflags: StfsVolumeDescriptorFlags\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let __binrw_generated_endian_file_table_block_count = binrw::Endian::Little;
                let mut file_table_block_count: u16 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_endian_file_table_block_count,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'file_table_block_count' in StfsVolumeDescriptor"
                                .into(),
                            line: 1084u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   1083 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mlittle\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m1084\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mfile_table_block_count: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu16\u{1b}[39m\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let __binrw_generated_endian_file_table_block_num = binrw::Endian::Little;
                let mut file_table_block_num: Block = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_endian_file_table_block_num,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'file_table_block_num' in StfsVolumeDescriptor"
                                .into(),
                            line: 1086u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   1085 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbrw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mlittle\u{1b}[39m\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m1086\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mfile_table_block_num: Block\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut top_hash_table_hash: [u8; 0x14] = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'top_hash_table_hash' in StfsVolumeDescriptor"
                                .into(),
                            line: 1087u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   \u{1b}[1m1087\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mtop_hash_table_hash: [\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m; \u{1b}[0m\u{1b}[1m\u{1b}[38;5;135m0x14\u{1b}[39m\u{1b}[0m\u{1b}[1m]\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut allocated_block_count: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'allocated_block_count' in StfsVolumeDescriptor"
                                .into(),
                            line: 1088u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   \u{1b}[1m1088\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mallocated_block_count: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut unallocated_block_count: u32 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'unallocated_block_count' in StfsVolumeDescriptor"
                                .into(),
                            line: 1089u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   \u{1b}[1m1089\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1munallocated_block_count: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_this = Self {
                    size,
                    version,
                    flags,
                    file_table_block_count,
                    file_table_block_num,
                    top_hash_table_hash,
                    allocated_block_count,
                    unallocated_block_count,
                };
                Ok(__binrw_this)
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for StfsVolumeDescriptor {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_this = self;
            let StfsVolumeDescriptor {
                ref size,
                ref version,
                ref flags,
                ref file_table_block_count,
                ref file_table_block_num,
                ref top_hash_table_hash,
                ref allocated_block_count,
                ref unallocated_block_count,
            } = self;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_size: <u8 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &size,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_size,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_version: <u8 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &version,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_version,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_flags: <StfsVolumeDescriptorFlags as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &flags,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_flags,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_file_table_block_count: <u16 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &file_table_block_count,
                __binrw_generated_var_writer,
                binrw::Endian::Little,
                __binrw_generated_args_file_table_block_count,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_file_table_block_num: <Block as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &file_table_block_num,
                __binrw_generated_var_writer,
                binrw::Endian::Little,
                __binrw_generated_args_file_table_block_num,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_top_hash_table_hash: <[u8; 0x14] as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &top_hash_table_hash,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_top_hash_table_hash,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_allocated_block_count: <u32 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &allocated_block_count,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_allocated_block_count,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_unallocated_block_count: <u32 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &unallocated_block_count,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_unallocated_block_count,
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::default::Default for StfsVolumeDescriptor {
        #[inline]
        fn default() -> StfsVolumeDescriptor {
            StfsVolumeDescriptor {
                size: ::core::default::Default::default(),
                version: ::core::default::Default::default(),
                flags: ::core::default::Default::default(),
                file_table_block_count: ::core::default::Default::default(),
                file_table_block_num: ::core::default::Default::default(),
                top_hash_table_hash: ::core::default::Default::default(),
                allocated_block_count: ::core::default::Default::default(),
                unallocated_block_count: ::core::default::Default::default(),
            }
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for StfsVolumeDescriptor {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            let names: &'static _ = &[
                "size",
                "version",
                "flags",
                "file_table_block_count",
                "file_table_block_num",
                "top_hash_table_hash",
                "allocated_block_count",
                "unallocated_block_count",
            ];
            let values: &[&dyn ::core::fmt::Debug] = &[
                &self.size,
                &self.version,
                &self.flags,
                &self.file_table_block_count,
                &self.file_table_block_num,
                &self.top_hash_table_hash,
                &self.allocated_block_count,
                &&self.unallocated_block_count,
            ];
            ::core::fmt::Formatter::debug_struct_fields_finish(
                f,
                "StfsVolumeDescriptor",
                names,
                values,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for StfsVolumeDescriptor {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "StfsVolumeDescriptor",
                    false as usize + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "size",
                    &self.size,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "version",
                    &self.version,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "flags",
                    &self.flags,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "file_table_block_count",
                    &self.file_table_block_count,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "file_table_block_num",
                    &self.file_table_block_num,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "top_hash_table_hash",
                    &self.top_hash_table_hash,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "allocated_block_count",
                    &self.allocated_block_count,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "unallocated_block_count",
                    &self.unallocated_block_count,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    enum AssetSubcategory {
        CarryableCarryable = 0x44c,
        CostumeCasualSuit = 0x68,
        CostumeCostume = 0x69,
        CostumeFormalSuit = 0x67,
        CostumeLongDress = 0x65,
        CostumeShortDress = 100,
        EarringsDanglers = 0x387,
        EarringsLargehoops = 0x38b,
        EarringsSingleDangler = 0x386,
        EarringsSingleLargeHoop = 0x38a,
        EarringsSingleSmallHoop = 0x388,
        EarringsSingleStud = 900,
        EarringsSmallHoops = 0x389,
        EarringsStuds = 0x385,
        GlassesCostume = 0x2be,
        GlassesGlasses = 700,
        GlassesSunglasses = 0x2bd,
        GlovesFingerless = 600,
        GlovesFullFingered = 0x259,
        HatBaseballCap = 0x1f6,
        HatBeanie = 500,
        HatBearskin = 0x1fc,
        HatBrimmed = 0x1f8,
        HatCostume = 0x1fb,
        HatFez = 0x1f9,
        HatFlatCap = 0x1f5,
        HatHeadwrap = 0x1fa,
        HatHelmet = 0x1fd,
        HatPeakCap = 0x1f7,
        RingLast = 0x3ea,
        RingLeft = 0x3e9,
        RingRight = 0x3e8,
        ShirtCoat = 210,
        ShirtHoodie = 0xd0,
        ShirtJacket = 0xd1,
        ShirtLongSleeveShirt = 0xce,
        ShirtLongSleeveTee = 0xcc,
        ShirtPolo = 0xcb,
        ShirtShortSleeveShirt = 0xcd,
        ShirtSportsTee = 200,
        ShirtSweater = 0xcf,
        ShirtTee = 0xc9,
        ShirtVest = 0xca,
        ShoesCostume = 0x197,
        ShoesFormal = 0x193,
        ShoesHeels = 0x191,
        ShoesHighBoots = 0x196,
        ShoesPumps = 0x192,
        ShoesSandals = 400,
        ShoesShortBoots = 0x195,
        ShoesTrainers = 0x194,
        TrousersCargo = 0x131,
        TrousersHotpants = 300,
        TrousersJeans = 0x132,
        TrousersKilt = 0x134,
        TrousersLeggings = 0x12f,
        TrousersLongShorts = 0x12e,
        TrousersLongSkirt = 0x135,
        TrousersShorts = 0x12d,
        TrousersShortSkirt = 0x133,
        TrousersTrousers = 0x130,
        WristwearBands = 0x322,
        WristwearBracelet = 800,
        WristwearSweatbands = 0x323,
        WristwearWatch = 0x321,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for AssetSubcategory {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_temp: u32 = binrw::BinRead::read_options(
                    __binrw_generated_var_reader,
                    __binrw_generated_var_endian,
                    (),
                )?;
                if __binrw_temp == Self::CarryableCarryable as u32 {
                    Ok(Self::CarryableCarryable)
                } else if __binrw_temp == Self::CostumeCasualSuit as u32 {
                    Ok(Self::CostumeCasualSuit)
                } else if __binrw_temp == Self::CostumeCostume as u32 {
                    Ok(Self::CostumeCostume)
                } else if __binrw_temp == Self::CostumeFormalSuit as u32 {
                    Ok(Self::CostumeFormalSuit)
                } else if __binrw_temp == Self::CostumeLongDress as u32 {
                    Ok(Self::CostumeLongDress)
                } else if __binrw_temp == Self::CostumeShortDress as u32 {
                    Ok(Self::CostumeShortDress)
                } else if __binrw_temp == Self::EarringsDanglers as u32 {
                    Ok(Self::EarringsDanglers)
                } else if __binrw_temp == Self::EarringsLargehoops as u32 {
                    Ok(Self::EarringsLargehoops)
                } else if __binrw_temp == Self::EarringsSingleDangler as u32 {
                    Ok(Self::EarringsSingleDangler)
                } else if __binrw_temp == Self::EarringsSingleLargeHoop as u32 {
                    Ok(Self::EarringsSingleLargeHoop)
                } else if __binrw_temp == Self::EarringsSingleSmallHoop as u32 {
                    Ok(Self::EarringsSingleSmallHoop)
                } else if __binrw_temp == Self::EarringsSingleStud as u32 {
                    Ok(Self::EarringsSingleStud)
                } else if __binrw_temp == Self::EarringsSmallHoops as u32 {
                    Ok(Self::EarringsSmallHoops)
                } else if __binrw_temp == Self::EarringsStuds as u32 {
                    Ok(Self::EarringsStuds)
                } else if __binrw_temp == Self::GlassesCostume as u32 {
                    Ok(Self::GlassesCostume)
                } else if __binrw_temp == Self::GlassesGlasses as u32 {
                    Ok(Self::GlassesGlasses)
                } else if __binrw_temp == Self::GlassesSunglasses as u32 {
                    Ok(Self::GlassesSunglasses)
                } else if __binrw_temp == Self::GlovesFingerless as u32 {
                    Ok(Self::GlovesFingerless)
                } else if __binrw_temp == Self::GlovesFullFingered as u32 {
                    Ok(Self::GlovesFullFingered)
                } else if __binrw_temp == Self::HatBaseballCap as u32 {
                    Ok(Self::HatBaseballCap)
                } else if __binrw_temp == Self::HatBeanie as u32 {
                    Ok(Self::HatBeanie)
                } else if __binrw_temp == Self::HatBearskin as u32 {
                    Ok(Self::HatBearskin)
                } else if __binrw_temp == Self::HatBrimmed as u32 {
                    Ok(Self::HatBrimmed)
                } else if __binrw_temp == Self::HatCostume as u32 {
                    Ok(Self::HatCostume)
                } else if __binrw_temp == Self::HatFez as u32 {
                    Ok(Self::HatFez)
                } else if __binrw_temp == Self::HatFlatCap as u32 {
                    Ok(Self::HatFlatCap)
                } else if __binrw_temp == Self::HatHeadwrap as u32 {
                    Ok(Self::HatHeadwrap)
                } else if __binrw_temp == Self::HatHelmet as u32 {
                    Ok(Self::HatHelmet)
                } else if __binrw_temp == Self::HatPeakCap as u32 {
                    Ok(Self::HatPeakCap)
                } else if __binrw_temp == Self::RingLast as u32 {
                    Ok(Self::RingLast)
                } else if __binrw_temp == Self::RingLeft as u32 {
                    Ok(Self::RingLeft)
                } else if __binrw_temp == Self::RingRight as u32 {
                    Ok(Self::RingRight)
                } else if __binrw_temp == Self::ShirtCoat as u32 {
                    Ok(Self::ShirtCoat)
                } else if __binrw_temp == Self::ShirtHoodie as u32 {
                    Ok(Self::ShirtHoodie)
                } else if __binrw_temp == Self::ShirtJacket as u32 {
                    Ok(Self::ShirtJacket)
                } else if __binrw_temp == Self::ShirtLongSleeveShirt as u32 {
                    Ok(Self::ShirtLongSleeveShirt)
                } else if __binrw_temp == Self::ShirtLongSleeveTee as u32 {
                    Ok(Self::ShirtLongSleeveTee)
                } else if __binrw_temp == Self::ShirtPolo as u32 {
                    Ok(Self::ShirtPolo)
                } else if __binrw_temp == Self::ShirtShortSleeveShirt as u32 {
                    Ok(Self::ShirtShortSleeveShirt)
                } else if __binrw_temp == Self::ShirtSportsTee as u32 {
                    Ok(Self::ShirtSportsTee)
                } else if __binrw_temp == Self::ShirtSweater as u32 {
                    Ok(Self::ShirtSweater)
                } else if __binrw_temp == Self::ShirtTee as u32 {
                    Ok(Self::ShirtTee)
                } else if __binrw_temp == Self::ShirtVest as u32 {
                    Ok(Self::ShirtVest)
                } else if __binrw_temp == Self::ShoesCostume as u32 {
                    Ok(Self::ShoesCostume)
                } else if __binrw_temp == Self::ShoesFormal as u32 {
                    Ok(Self::ShoesFormal)
                } else if __binrw_temp == Self::ShoesHeels as u32 {
                    Ok(Self::ShoesHeels)
                } else if __binrw_temp == Self::ShoesHighBoots as u32 {
                    Ok(Self::ShoesHighBoots)
                } else if __binrw_temp == Self::ShoesPumps as u32 {
                    Ok(Self::ShoesPumps)
                } else if __binrw_temp == Self::ShoesSandals as u32 {
                    Ok(Self::ShoesSandals)
                } else if __binrw_temp == Self::ShoesShortBoots as u32 {
                    Ok(Self::ShoesShortBoots)
                } else if __binrw_temp == Self::ShoesTrainers as u32 {
                    Ok(Self::ShoesTrainers)
                } else if __binrw_temp == Self::TrousersCargo as u32 {
                    Ok(Self::TrousersCargo)
                } else if __binrw_temp == Self::TrousersHotpants as u32 {
                    Ok(Self::TrousersHotpants)
                } else if __binrw_temp == Self::TrousersJeans as u32 {
                    Ok(Self::TrousersJeans)
                } else if __binrw_temp == Self::TrousersKilt as u32 {
                    Ok(Self::TrousersKilt)
                } else if __binrw_temp == Self::TrousersLeggings as u32 {
                    Ok(Self::TrousersLeggings)
                } else if __binrw_temp == Self::TrousersLongShorts as u32 {
                    Ok(Self::TrousersLongShorts)
                } else if __binrw_temp == Self::TrousersLongSkirt as u32 {
                    Ok(Self::TrousersLongSkirt)
                } else if __binrw_temp == Self::TrousersShorts as u32 {
                    Ok(Self::TrousersShorts)
                } else if __binrw_temp == Self::TrousersShortSkirt as u32 {
                    Ok(Self::TrousersShortSkirt)
                } else if __binrw_temp == Self::TrousersTrousers as u32 {
                    Ok(Self::TrousersTrousers)
                } else if __binrw_temp == Self::WristwearBands as u32 {
                    Ok(Self::WristwearBands)
                } else if __binrw_temp == Self::WristwearBracelet as u32 {
                    Ok(Self::WristwearBracelet)
                } else if __binrw_temp == Self::WristwearSweatbands as u32 {
                    Ok(Self::WristwearSweatbands)
                } else if __binrw_temp == Self::WristwearWatch as u32 {
                    Ok(Self::WristwearWatch)
                } else {
                    Err(
                        binrw::error::ContextExt::with_context(
                            binrw::Error::NoVariantMatch {
                                pos: __binrw_generated_position_temp,
                            },
                            binrw::error::BacktraceFrame::Message({
                                extern crate alloc;
                                {
                                    let res = ::alloc::fmt::format(
                                        format_args!(
                                            "Unexpected value for enum: {0:?}",
                                            __binrw_temp,
                                        ),
                                    );
                                    res
                                }
                                    .into()
                            }),
                        ),
                    )
                }
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for AssetSubcategory {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            binrw::BinWrite::write_options(
                &(match self {
                    Self::CarryableCarryable => Self::CarryableCarryable,
                    Self::CostumeCasualSuit => Self::CostumeCasualSuit,
                    Self::CostumeCostume => Self::CostumeCostume,
                    Self::CostumeFormalSuit => Self::CostumeFormalSuit,
                    Self::CostumeLongDress => Self::CostumeLongDress,
                    Self::CostumeShortDress => Self::CostumeShortDress,
                    Self::EarringsDanglers => Self::EarringsDanglers,
                    Self::EarringsLargehoops => Self::EarringsLargehoops,
                    Self::EarringsSingleDangler => Self::EarringsSingleDangler,
                    Self::EarringsSingleLargeHoop => Self::EarringsSingleLargeHoop,
                    Self::EarringsSingleSmallHoop => Self::EarringsSingleSmallHoop,
                    Self::EarringsSingleStud => Self::EarringsSingleStud,
                    Self::EarringsSmallHoops => Self::EarringsSmallHoops,
                    Self::EarringsStuds => Self::EarringsStuds,
                    Self::GlassesCostume => Self::GlassesCostume,
                    Self::GlassesGlasses => Self::GlassesGlasses,
                    Self::GlassesSunglasses => Self::GlassesSunglasses,
                    Self::GlovesFingerless => Self::GlovesFingerless,
                    Self::GlovesFullFingered => Self::GlovesFullFingered,
                    Self::HatBaseballCap => Self::HatBaseballCap,
                    Self::HatBeanie => Self::HatBeanie,
                    Self::HatBearskin => Self::HatBearskin,
                    Self::HatBrimmed => Self::HatBrimmed,
                    Self::HatCostume => Self::HatCostume,
                    Self::HatFez => Self::HatFez,
                    Self::HatFlatCap => Self::HatFlatCap,
                    Self::HatHeadwrap => Self::HatHeadwrap,
                    Self::HatHelmet => Self::HatHelmet,
                    Self::HatPeakCap => Self::HatPeakCap,
                    Self::RingLast => Self::RingLast,
                    Self::RingLeft => Self::RingLeft,
                    Self::RingRight => Self::RingRight,
                    Self::ShirtCoat => Self::ShirtCoat,
                    Self::ShirtHoodie => Self::ShirtHoodie,
                    Self::ShirtJacket => Self::ShirtJacket,
                    Self::ShirtLongSleeveShirt => Self::ShirtLongSleeveShirt,
                    Self::ShirtLongSleeveTee => Self::ShirtLongSleeveTee,
                    Self::ShirtPolo => Self::ShirtPolo,
                    Self::ShirtShortSleeveShirt => Self::ShirtShortSleeveShirt,
                    Self::ShirtSportsTee => Self::ShirtSportsTee,
                    Self::ShirtSweater => Self::ShirtSweater,
                    Self::ShirtTee => Self::ShirtTee,
                    Self::ShirtVest => Self::ShirtVest,
                    Self::ShoesCostume => Self::ShoesCostume,
                    Self::ShoesFormal => Self::ShoesFormal,
                    Self::ShoesHeels => Self::ShoesHeels,
                    Self::ShoesHighBoots => Self::ShoesHighBoots,
                    Self::ShoesPumps => Self::ShoesPumps,
                    Self::ShoesSandals => Self::ShoesSandals,
                    Self::ShoesShortBoots => Self::ShoesShortBoots,
                    Self::ShoesTrainers => Self::ShoesTrainers,
                    Self::TrousersCargo => Self::TrousersCargo,
                    Self::TrousersHotpants => Self::TrousersHotpants,
                    Self::TrousersJeans => Self::TrousersJeans,
                    Self::TrousersKilt => Self::TrousersKilt,
                    Self::TrousersLeggings => Self::TrousersLeggings,
                    Self::TrousersLongShorts => Self::TrousersLongShorts,
                    Self::TrousersLongSkirt => Self::TrousersLongSkirt,
                    Self::TrousersShorts => Self::TrousersShorts,
                    Self::TrousersShortSkirt => Self::TrousersShortSkirt,
                    Self::TrousersTrousers => Self::TrousersTrousers,
                    Self::WristwearBands => Self::WristwearBands,
                    Self::WristwearBracelet => Self::WristwearBracelet,
                    Self::WristwearSweatbands => Self::WristwearSweatbands,
                    Self::WristwearWatch => Self::WristwearWatch,
                } as u32),
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                (),
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for AssetSubcategory {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    AssetSubcategory::CarryableCarryable => "CarryableCarryable",
                    AssetSubcategory::CostumeCasualSuit => "CostumeCasualSuit",
                    AssetSubcategory::CostumeCostume => "CostumeCostume",
                    AssetSubcategory::CostumeFormalSuit => "CostumeFormalSuit",
                    AssetSubcategory::CostumeLongDress => "CostumeLongDress",
                    AssetSubcategory::CostumeShortDress => "CostumeShortDress",
                    AssetSubcategory::EarringsDanglers => "EarringsDanglers",
                    AssetSubcategory::EarringsLargehoops => "EarringsLargehoops",
                    AssetSubcategory::EarringsSingleDangler => "EarringsSingleDangler",
                    AssetSubcategory::EarringsSingleLargeHoop => {
                        "EarringsSingleLargeHoop"
                    }
                    AssetSubcategory::EarringsSingleSmallHoop => {
                        "EarringsSingleSmallHoop"
                    }
                    AssetSubcategory::EarringsSingleStud => "EarringsSingleStud",
                    AssetSubcategory::EarringsSmallHoops => "EarringsSmallHoops",
                    AssetSubcategory::EarringsStuds => "EarringsStuds",
                    AssetSubcategory::GlassesCostume => "GlassesCostume",
                    AssetSubcategory::GlassesGlasses => "GlassesGlasses",
                    AssetSubcategory::GlassesSunglasses => "GlassesSunglasses",
                    AssetSubcategory::GlovesFingerless => "GlovesFingerless",
                    AssetSubcategory::GlovesFullFingered => "GlovesFullFingered",
                    AssetSubcategory::HatBaseballCap => "HatBaseballCap",
                    AssetSubcategory::HatBeanie => "HatBeanie",
                    AssetSubcategory::HatBearskin => "HatBearskin",
                    AssetSubcategory::HatBrimmed => "HatBrimmed",
                    AssetSubcategory::HatCostume => "HatCostume",
                    AssetSubcategory::HatFez => "HatFez",
                    AssetSubcategory::HatFlatCap => "HatFlatCap",
                    AssetSubcategory::HatHeadwrap => "HatHeadwrap",
                    AssetSubcategory::HatHelmet => "HatHelmet",
                    AssetSubcategory::HatPeakCap => "HatPeakCap",
                    AssetSubcategory::RingLast => "RingLast",
                    AssetSubcategory::RingLeft => "RingLeft",
                    AssetSubcategory::RingRight => "RingRight",
                    AssetSubcategory::ShirtCoat => "ShirtCoat",
                    AssetSubcategory::ShirtHoodie => "ShirtHoodie",
                    AssetSubcategory::ShirtJacket => "ShirtJacket",
                    AssetSubcategory::ShirtLongSleeveShirt => "ShirtLongSleeveShirt",
                    AssetSubcategory::ShirtLongSleeveTee => "ShirtLongSleeveTee",
                    AssetSubcategory::ShirtPolo => "ShirtPolo",
                    AssetSubcategory::ShirtShortSleeveShirt => "ShirtShortSleeveShirt",
                    AssetSubcategory::ShirtSportsTee => "ShirtSportsTee",
                    AssetSubcategory::ShirtSweater => "ShirtSweater",
                    AssetSubcategory::ShirtTee => "ShirtTee",
                    AssetSubcategory::ShirtVest => "ShirtVest",
                    AssetSubcategory::ShoesCostume => "ShoesCostume",
                    AssetSubcategory::ShoesFormal => "ShoesFormal",
                    AssetSubcategory::ShoesHeels => "ShoesHeels",
                    AssetSubcategory::ShoesHighBoots => "ShoesHighBoots",
                    AssetSubcategory::ShoesPumps => "ShoesPumps",
                    AssetSubcategory::ShoesSandals => "ShoesSandals",
                    AssetSubcategory::ShoesShortBoots => "ShoesShortBoots",
                    AssetSubcategory::ShoesTrainers => "ShoesTrainers",
                    AssetSubcategory::TrousersCargo => "TrousersCargo",
                    AssetSubcategory::TrousersHotpants => "TrousersHotpants",
                    AssetSubcategory::TrousersJeans => "TrousersJeans",
                    AssetSubcategory::TrousersKilt => "TrousersKilt",
                    AssetSubcategory::TrousersLeggings => "TrousersLeggings",
                    AssetSubcategory::TrousersLongShorts => "TrousersLongShorts",
                    AssetSubcategory::TrousersLongSkirt => "TrousersLongSkirt",
                    AssetSubcategory::TrousersShorts => "TrousersShorts",
                    AssetSubcategory::TrousersShortSkirt => "TrousersShortSkirt",
                    AssetSubcategory::TrousersTrousers => "TrousersTrousers",
                    AssetSubcategory::WristwearBands => "WristwearBands",
                    AssetSubcategory::WristwearBracelet => "WristwearBracelet",
                    AssetSubcategory::WristwearSweatbands => "WristwearSweatbands",
                    AssetSubcategory::WristwearWatch => "WristwearWatch",
                },
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for AssetSubcategory {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    AssetSubcategory::CarryableCarryable => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            0u32,
                            "CarryableCarryable",
                        )
                    }
                    AssetSubcategory::CostumeCasualSuit => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            1u32,
                            "CostumeCasualSuit",
                        )
                    }
                    AssetSubcategory::CostumeCostume => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            2u32,
                            "CostumeCostume",
                        )
                    }
                    AssetSubcategory::CostumeFormalSuit => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            3u32,
                            "CostumeFormalSuit",
                        )
                    }
                    AssetSubcategory::CostumeLongDress => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            4u32,
                            "CostumeLongDress",
                        )
                    }
                    AssetSubcategory::CostumeShortDress => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            5u32,
                            "CostumeShortDress",
                        )
                    }
                    AssetSubcategory::EarringsDanglers => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            6u32,
                            "EarringsDanglers",
                        )
                    }
                    AssetSubcategory::EarringsLargehoops => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            7u32,
                            "EarringsLargehoops",
                        )
                    }
                    AssetSubcategory::EarringsSingleDangler => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            8u32,
                            "EarringsSingleDangler",
                        )
                    }
                    AssetSubcategory::EarringsSingleLargeHoop => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            9u32,
                            "EarringsSingleLargeHoop",
                        )
                    }
                    AssetSubcategory::EarringsSingleSmallHoop => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            10u32,
                            "EarringsSingleSmallHoop",
                        )
                    }
                    AssetSubcategory::EarringsSingleStud => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            11u32,
                            "EarringsSingleStud",
                        )
                    }
                    AssetSubcategory::EarringsSmallHoops => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            12u32,
                            "EarringsSmallHoops",
                        )
                    }
                    AssetSubcategory::EarringsStuds => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            13u32,
                            "EarringsStuds",
                        )
                    }
                    AssetSubcategory::GlassesCostume => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            14u32,
                            "GlassesCostume",
                        )
                    }
                    AssetSubcategory::GlassesGlasses => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            15u32,
                            "GlassesGlasses",
                        )
                    }
                    AssetSubcategory::GlassesSunglasses => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            16u32,
                            "GlassesSunglasses",
                        )
                    }
                    AssetSubcategory::GlovesFingerless => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            17u32,
                            "GlovesFingerless",
                        )
                    }
                    AssetSubcategory::GlovesFullFingered => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            18u32,
                            "GlovesFullFingered",
                        )
                    }
                    AssetSubcategory::HatBaseballCap => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            19u32,
                            "HatBaseballCap",
                        )
                    }
                    AssetSubcategory::HatBeanie => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            20u32,
                            "HatBeanie",
                        )
                    }
                    AssetSubcategory::HatBearskin => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            21u32,
                            "HatBearskin",
                        )
                    }
                    AssetSubcategory::HatBrimmed => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            22u32,
                            "HatBrimmed",
                        )
                    }
                    AssetSubcategory::HatCostume => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            23u32,
                            "HatCostume",
                        )
                    }
                    AssetSubcategory::HatFez => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            24u32,
                            "HatFez",
                        )
                    }
                    AssetSubcategory::HatFlatCap => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            25u32,
                            "HatFlatCap",
                        )
                    }
                    AssetSubcategory::HatHeadwrap => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            26u32,
                            "HatHeadwrap",
                        )
                    }
                    AssetSubcategory::HatHelmet => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            27u32,
                            "HatHelmet",
                        )
                    }
                    AssetSubcategory::HatPeakCap => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            28u32,
                            "HatPeakCap",
                        )
                    }
                    AssetSubcategory::RingLast => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            29u32,
                            "RingLast",
                        )
                    }
                    AssetSubcategory::RingLeft => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            30u32,
                            "RingLeft",
                        )
                    }
                    AssetSubcategory::RingRight => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            31u32,
                            "RingRight",
                        )
                    }
                    AssetSubcategory::ShirtCoat => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            32u32,
                            "ShirtCoat",
                        )
                    }
                    AssetSubcategory::ShirtHoodie => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            33u32,
                            "ShirtHoodie",
                        )
                    }
                    AssetSubcategory::ShirtJacket => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            34u32,
                            "ShirtJacket",
                        )
                    }
                    AssetSubcategory::ShirtLongSleeveShirt => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            35u32,
                            "ShirtLongSleeveShirt",
                        )
                    }
                    AssetSubcategory::ShirtLongSleeveTee => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            36u32,
                            "ShirtLongSleeveTee",
                        )
                    }
                    AssetSubcategory::ShirtPolo => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            37u32,
                            "ShirtPolo",
                        )
                    }
                    AssetSubcategory::ShirtShortSleeveShirt => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            38u32,
                            "ShirtShortSleeveShirt",
                        )
                    }
                    AssetSubcategory::ShirtSportsTee => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            39u32,
                            "ShirtSportsTee",
                        )
                    }
                    AssetSubcategory::ShirtSweater => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            40u32,
                            "ShirtSweater",
                        )
                    }
                    AssetSubcategory::ShirtTee => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            41u32,
                            "ShirtTee",
                        )
                    }
                    AssetSubcategory::ShirtVest => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            42u32,
                            "ShirtVest",
                        )
                    }
                    AssetSubcategory::ShoesCostume => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            43u32,
                            "ShoesCostume",
                        )
                    }
                    AssetSubcategory::ShoesFormal => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            44u32,
                            "ShoesFormal",
                        )
                    }
                    AssetSubcategory::ShoesHeels => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            45u32,
                            "ShoesHeels",
                        )
                    }
                    AssetSubcategory::ShoesHighBoots => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            46u32,
                            "ShoesHighBoots",
                        )
                    }
                    AssetSubcategory::ShoesPumps => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            47u32,
                            "ShoesPumps",
                        )
                    }
                    AssetSubcategory::ShoesSandals => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            48u32,
                            "ShoesSandals",
                        )
                    }
                    AssetSubcategory::ShoesShortBoots => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            49u32,
                            "ShoesShortBoots",
                        )
                    }
                    AssetSubcategory::ShoesTrainers => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            50u32,
                            "ShoesTrainers",
                        )
                    }
                    AssetSubcategory::TrousersCargo => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            51u32,
                            "TrousersCargo",
                        )
                    }
                    AssetSubcategory::TrousersHotpants => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            52u32,
                            "TrousersHotpants",
                        )
                    }
                    AssetSubcategory::TrousersJeans => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            53u32,
                            "TrousersJeans",
                        )
                    }
                    AssetSubcategory::TrousersKilt => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            54u32,
                            "TrousersKilt",
                        )
                    }
                    AssetSubcategory::TrousersLeggings => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            55u32,
                            "TrousersLeggings",
                        )
                    }
                    AssetSubcategory::TrousersLongShorts => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            56u32,
                            "TrousersLongShorts",
                        )
                    }
                    AssetSubcategory::TrousersLongSkirt => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            57u32,
                            "TrousersLongSkirt",
                        )
                    }
                    AssetSubcategory::TrousersShorts => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            58u32,
                            "TrousersShorts",
                        )
                    }
                    AssetSubcategory::TrousersShortSkirt => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            59u32,
                            "TrousersShortSkirt",
                        )
                    }
                    AssetSubcategory::TrousersTrousers => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            60u32,
                            "TrousersTrousers",
                        )
                    }
                    AssetSubcategory::WristwearBands => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            61u32,
                            "WristwearBands",
                        )
                    }
                    AssetSubcategory::WristwearBracelet => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            62u32,
                            "WristwearBracelet",
                        )
                    }
                    AssetSubcategory::WristwearSweatbands => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            63u32,
                            "WristwearSweatbands",
                        )
                    }
                    AssetSubcategory::WristwearWatch => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetSubcategory",
                            64u32,
                            "WristwearWatch",
                        )
                    }
                }
            }
        }
    };
    #[automatically_derived]
    impl ::core::marker::Copy for AssetSubcategory {}
    #[automatically_derived]
    impl ::core::clone::Clone for AssetSubcategory {
        #[inline]
        fn clone(&self) -> AssetSubcategory {
            *self
        }
    }
    #[automatically_derived]
    impl ::core::cmp::Eq for AssetSubcategory {
        #[inline]
        #[doc(hidden)]
        #[coverage(off)]
        fn assert_receiver_is_total_eq(&self) -> () {}
    }
    #[automatically_derived]
    impl ::core::marker::StructuralPartialEq for AssetSubcategory {}
    #[automatically_derived]
    impl ::core::cmp::PartialEq for AssetSubcategory {
        #[inline]
        fn eq(&self, other: &AssetSubcategory) -> bool {
            let __self_tag = ::core::intrinsics::discriminant_value(self);
            let __arg1_tag = ::core::intrinsics::discriminant_value(other);
            __self_tag == __arg1_tag
        }
    }
    enum BinaryAssetType {
        Component = 1,
        Texture = 2,
        ShapeOverride = 3,
        Animation = 4,
        ShapeOverridePost = 5,
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for BinaryAssetType {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    BinaryAssetType::Component => "Component",
                    BinaryAssetType::Texture => "Texture",
                    BinaryAssetType::ShapeOverride => "ShapeOverride",
                    BinaryAssetType::Animation => "Animation",
                    BinaryAssetType::ShapeOverridePost => "ShapeOverridePost",
                },
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for BinaryAssetType {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    BinaryAssetType::Component => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "BinaryAssetType",
                            0u32,
                            "Component",
                        )
                    }
                    BinaryAssetType::Texture => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "BinaryAssetType",
                            1u32,
                            "Texture",
                        )
                    }
                    BinaryAssetType::ShapeOverride => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "BinaryAssetType",
                            2u32,
                            "ShapeOverride",
                        )
                    }
                    BinaryAssetType::Animation => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "BinaryAssetType",
                            3u32,
                            "Animation",
                        )
                    }
                    BinaryAssetType::ShapeOverridePost => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "BinaryAssetType",
                            4u32,
                            "ShapeOverridePost",
                        )
                    }
                }
            }
        }
    };
    enum SkeletonVersion {
        Nxe = 1,
        Natal,
        NxeAndNatal,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for SkeletonVersion {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_temp: u8 = binrw::BinRead::read_options(
                    __binrw_generated_var_reader,
                    __binrw_generated_var_endian,
                    (),
                )?;
                if __binrw_temp == Self::Nxe as u8 {
                    Ok(Self::Nxe)
                } else if __binrw_temp == Self::Natal as u8 {
                    Ok(Self::Natal)
                } else if __binrw_temp == Self::NxeAndNatal as u8 {
                    Ok(Self::NxeAndNatal)
                } else {
                    Err(
                        binrw::error::ContextExt::with_context(
                            binrw::Error::NoVariantMatch {
                                pos: __binrw_generated_position_temp,
                            },
                            binrw::error::BacktraceFrame::Message({
                                extern crate alloc;
                                {
                                    let res = ::alloc::fmt::format(
                                        format_args!(
                                            "Unexpected value for enum: {0:?}",
                                            __binrw_temp,
                                        ),
                                    );
                                    res
                                }
                                    .into()
                            }),
                        ),
                    )
                }
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    impl binrw::meta::ReadEndian for SkeletonVersion {
        const ENDIAN: binrw::meta::EndianKind = <(u8) as binrw::meta::ReadEndian>::ENDIAN;
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for SkeletonVersion {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            binrw::BinWrite::write_options(
                &(match self {
                    Self::Nxe => Self::Nxe,
                    Self::Natal => Self::Natal,
                    Self::NxeAndNatal => Self::NxeAndNatal,
                } as u8),
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                (),
            )?;
            Ok(())
        }
    }
    impl binrw::meta::WriteEndian for SkeletonVersion {
        const ENDIAN: binrw::meta::EndianKind = <(u8) as binrw::meta::WriteEndian>::ENDIAN;
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for SkeletonVersion {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    SkeletonVersion::Nxe => "Nxe",
                    SkeletonVersion::Natal => "Natal",
                    SkeletonVersion::NxeAndNatal => "NxeAndNatal",
                },
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for SkeletonVersion {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    SkeletonVersion::Nxe => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "SkeletonVersion",
                            0u32,
                            "Nxe",
                        )
                    }
                    SkeletonVersion::Natal => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "SkeletonVersion",
                            1u32,
                            "Natal",
                        )
                    }
                    SkeletonVersion::NxeAndNatal => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "SkeletonVersion",
                            2u32,
                            "NxeAndNatal",
                        )
                    }
                }
            }
        }
    };
    enum AssetGender {
        Male = 1,
        Female,
        Both,
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for AssetGender {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_temp: u8 = binrw::BinRead::read_options(
                    __binrw_generated_var_reader,
                    __binrw_generated_var_endian,
                    (),
                )?;
                if __binrw_temp == Self::Male as u8 {
                    Ok(Self::Male)
                } else if __binrw_temp == Self::Female as u8 {
                    Ok(Self::Female)
                } else if __binrw_temp == Self::Both as u8 {
                    Ok(Self::Both)
                } else {
                    Err(
                        binrw::error::ContextExt::with_context(
                            binrw::Error::NoVariantMatch {
                                pos: __binrw_generated_position_temp,
                            },
                            binrw::error::BacktraceFrame::Message({
                                extern crate alloc;
                                {
                                    let res = ::alloc::fmt::format(
                                        format_args!(
                                            "Unexpected value for enum: {0:?}",
                                            __binrw_temp,
                                        ),
                                    );
                                    res
                                }
                                    .into()
                            }),
                        ),
                    )
                }
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    impl binrw::meta::ReadEndian for AssetGender {
        const ENDIAN: binrw::meta::EndianKind = <(u8) as binrw::meta::ReadEndian>::ENDIAN;
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for AssetGender {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            binrw::BinWrite::write_options(
                &(match self {
                    Self::Male => Self::Male,
                    Self::Female => Self::Female,
                    Self::Both => Self::Both,
                } as u8),
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                (),
            )?;
            Ok(())
        }
    }
    impl binrw::meta::WriteEndian for AssetGender {
        const ENDIAN: binrw::meta::EndianKind = <(u8) as binrw::meta::WriteEndian>::ENDIAN;
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for AssetGender {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    AssetGender::Male => "Male",
                    AssetGender::Female => "Female",
                    AssetGender::Both => "Both",
                },
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for AssetGender {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    AssetGender::Male => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetGender",
                            0u32,
                            "Male",
                        )
                    }
                    AssetGender::Female => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetGender",
                            1u32,
                            "Female",
                        )
                    }
                    AssetGender::Both => {
                        _serde::Serializer::serialize_unit_variant(
                            __serializer,
                            "AssetGender",
                            2u32,
                            "Both",
                        )
                    }
                }
            }
        }
    };
    pub struct SvodVolumeDescriptor {
        size: u8,
        block_cache_element_count: u8,
        worker_thread_processor: u8,
        worker_thread_priority: u8,
        root_hash: [u8; 0x14],
        flags: u8,
        /// Encoded as an int24
        data_block_count: u32,
        /// Encoded as an int24
        data_block_offset: u32,
        reserved: [u8; 5],
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinRead for SvodVolumeDescriptor {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn read_options<R: binrw::io::Read + binrw::io::Seek>(
            __binrw_generated_var_reader: &mut R,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<Self> {
            let __binrw_generated_var_reader = __binrw_generated_var_reader;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_reader,
            )?;
            (|| {
                let __binrw_generated_var_endian = __binrw_generated_var_endian;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut size: u8 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'size' in SvodVolumeDescriptor"
                                .into(),
                            line: 1214u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   \u{1b}[1m1214\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1msize: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut block_cache_element_count: u8 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'block_cache_element_count' in SvodVolumeDescriptor"
                                .into(),
                            line: 1215u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   \u{1b}[1m1215\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mblock_cache_element_count: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut worker_thread_processor: u8 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'worker_thread_processor' in SvodVolumeDescriptor"
                                .into(),
                            line: 1216u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   \u{1b}[1m1216\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mworker_thread_processor: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut worker_thread_priority: u8 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'worker_thread_priority' in SvodVolumeDescriptor"
                                .into(),
                            line: 1217u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   \u{1b}[1m1217\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mworker_thread_priority: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut root_hash: [u8; 0x14] = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'root_hash' in SvodVolumeDescriptor"
                                .into(),
                            line: 1218u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   \u{1b}[1m1218\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mroot_hash: [\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m; \u{1b}[0m\u{1b}[1m\u{1b}[38;5;135m0x14\u{1b}[39m\u{1b}[0m\u{1b}[1m]\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut flags: u8 = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'flags' in SvodVolumeDescriptor"
                                .into(),
                            line: 1219u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   \u{1b}[1m1219\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mflags: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::__private::parse_fn_type_hint(
                    binrw::helpers::read_u24,
                );
                let mut data_block_count: u32 = (|| {
                    __binrw_generated_read_function
                })()(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map(|v| -> u32 { v })
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'data_block_count' in SvodVolumeDescriptor"
                                .into(),
                            line: 1223u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   1220 |  \u{1b}[38;5;243m/// Encoded as an int24\u{1b}[39m\n   1221 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mparse_with\u{1b}[39m = \u{1b}[38;5;197mbinrw\u{1b}[39m::helpers::read_u24\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   1222 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39mwrite_with = binrw::helpers::write_u24\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m1223\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mdata_block_count: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::__private::parse_fn_type_hint(
                    binrw::helpers::read_u24,
                );
                let mut data_block_offset: u32 = (|| {
                    __binrw_generated_read_function
                })()(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map(|v| -> u32 { v })
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'data_block_offset' in SvodVolumeDescriptor"
                                .into(),
                            line: 1227u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   1224 |  \u{1b}[38;5;243m/// Encoded as an int24\u{1b}[39m\n   1225 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbr\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39m\u{1b}[38;5;197mparse_with\u{1b}[39m = \u{1b}[38;5;197mbinrw\u{1b}[39m::helpers::read_u24\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   1226 |  \u{1b}[38;5;197m#\u{1b}[39m\u{1b}[38;5;197m[\u{1b}[39m\u{1b}[38;5;197mbw\u{1b}[39m\u{1b}[38;5;197m(\u{1b}[39mwrite_with = binrw::helpers::write_u24\u{1b}[38;5;197m)\u{1b}[39m\u{1b}[38;5;197m]\u{1b}[39m\n   \u{1b}[1m1227\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mdata_block_offset: \u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu32\u{1b}[39m\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_generated_read_function = binrw::BinRead::read_options;
                let mut reserved: [u8; 5] = __binrw_generated_read_function(
                        __binrw_generated_var_reader,
                        __binrw_generated_var_endian,
                        <_ as binrw::__private::Required>::args(),
                    )
                    .map_err(|err| binrw::error::ContextExt::with_context(
                        err,
                        binrw::error::BacktraceFrame::Full {
                            message: "While parsing field 'reserved' in SvodVolumeDescriptor"
                                .into(),
                            line: 1228u32,
                            file: "stfs/src/parse.rs",
                            code: Some(
                                "  ┄─────╮\n   \u{1b}[1m1228\u{1b}[0m \u{1b}[1m⎬\u{1b}[0m  \u{1b}[1mreserved: [\u{1b}[0m\u{1b}[1m\u{1b}[38;5;197mu8\u{1b}[39m\u{1b}[0m\u{1b}[1m; \u{1b}[0m\u{1b}[1m\u{1b}[38;5;135m5\u{1b}[39m\u{1b}[0m\u{1b}[1m]\u{1b}[0m\n  ┄─────╯\n",
                            ),
                        },
                    ))?;
                let __binrw_this = Self {
                    size,
                    block_cache_element_count,
                    worker_thread_processor,
                    worker_thread_priority,
                    root_hash,
                    flags,
                    data_block_count,
                    data_block_offset,
                    reserved,
                };
                Ok(__binrw_this)
            })()
                .or_else(
                    binrw::__private::restore_position::<
                        binrw::Error,
                        _,
                        _,
                    >(__binrw_generated_var_reader, __binrw_generated_position_temp),
                )
        }
    }
    #[automatically_derived]
    #[allow(non_snake_case)]
    #[allow(clippy::redundant_closure_call)]
    impl binrw::BinWrite for SvodVolumeDescriptor {
        type Args<'__binrw_generated_args_lifetime> = ();
        fn write_options<W: binrw::io::Write + binrw::io::Seek>(
            &self,
            __binrw_generated_var_writer: &mut W,
            __binrw_generated_var_endian: binrw::Endian,
            __binrw_generated_var_arguments: Self::Args<'_>,
        ) -> binrw::BinResult<()> {
            let __binrw_generated_var_writer = __binrw_generated_var_writer;
            let __binrw_generated_position_temp = binrw::io::Seek::stream_position(
                __binrw_generated_var_writer,
            )?;
            let __binrw_this = self;
            let SvodVolumeDescriptor {
                ref size,
                ref block_cache_element_count,
                ref worker_thread_processor,
                ref worker_thread_priority,
                ref root_hash,
                ref flags,
                ref data_block_count,
                ref data_block_offset,
                ref reserved,
            } = self;
            let __binrw_generated_var_endian = __binrw_generated_var_endian;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_size: <u8 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &size,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_size,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_block_cache_element_count: <u8 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &block_cache_element_count,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_block_cache_element_count,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_worker_thread_processor: <u8 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &worker_thread_processor,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_worker_thread_processor,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_worker_thread_priority: <u8 as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &worker_thread_priority,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_worker_thread_priority,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_root_hash: <[u8; 0x14] as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &root_hash,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_root_hash,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_flags: <u8 as binrw::BinWrite>::Args<'_> = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &flags,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_flags,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::helpers::write_u24,
            );
            let __binrw_generated_args_data_block_count = binrw::__private::write_function_args_type_hint::<
                u32,
                _,
                _,
                _,
            >(
                __binrw_generated_write_function,
                <_ as binrw::__private::Required>::args(),
            );
            __binrw_generated_write_function(
                &data_block_count,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_data_block_count,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::helpers::write_u24,
            );
            let __binrw_generated_args_data_block_offset = binrw::__private::write_function_args_type_hint::<
                u32,
                _,
                _,
                _,
            >(
                __binrw_generated_write_function,
                <_ as binrw::__private::Required>::args(),
            );
            __binrw_generated_write_function(
                &data_block_offset,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_data_block_offset,
            )?;
            let __binrw_generated_write_function = binrw::__private::write_fn_type_hint(
                binrw::BinWrite::write_options,
            );
            let __binrw_generated_args_reserved: <[u8; 5] as binrw::BinWrite>::Args<
                '_,
            > = <_ as binrw::__private::Required>::args();
            __binrw_generated_write_function(
                &reserved,
                __binrw_generated_var_writer,
                __binrw_generated_var_endian,
                __binrw_generated_args_reserved,
            )?;
            Ok(())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for SvodVolumeDescriptor {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            let names: &'static _ = &[
                "size",
                "block_cache_element_count",
                "worker_thread_processor",
                "worker_thread_priority",
                "root_hash",
                "flags",
                "data_block_count",
                "data_block_offset",
                "reserved",
            ];
            let values: &[&dyn ::core::fmt::Debug] = &[
                &self.size,
                &self.block_cache_element_count,
                &self.worker_thread_processor,
                &self.worker_thread_priority,
                &self.root_hash,
                &self.flags,
                &self.data_block_count,
                &self.data_block_offset,
                &&self.reserved,
            ];
            ::core::fmt::Formatter::debug_struct_fields_finish(
                f,
                "SvodVolumeDescriptor",
                names,
                values,
            )
        }
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for SvodVolumeDescriptor {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = _serde::Serializer::serialize_struct(
                    __serializer,
                    "SvodVolumeDescriptor",
                    false as usize + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "size",
                    &self.size,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "block_cache_element_count",
                    &self.block_cache_element_count,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "worker_thread_processor",
                    &self.worker_thread_processor,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "worker_thread_priority",
                    &self.worker_thread_priority,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "root_hash",
                    &self.root_hash,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "flags",
                    &self.flags,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "data_block_count",
                    &self.data_block_count,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "data_block_offset",
                    &self.data_block_offset,
                )?;
                _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "reserved",
                    &self.reserved,
                )?;
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
}
mod sparse_reader {
    use std::io::Read;
    /// `SparseReader` helps reading data that is fragmented at various locations and
    /// potentially has chunks of differing sizes.
    ///
    /// # Example:
    ///
    /// ```compile_fail
    /// let first = [0u8, 1, 2, 3];
    /// let second = [4u8];
    /// let third = [5u8];
    /// let mappings = [first.as_slice(), second.as_slice(), third.as_slice()];
    /// let mut reader = SparseReader::new(&mappings);
    /// let mut output = [0u8; 6];
    /// assert!(matches!(reader.read(&mut output), Ok(6)));
    ///
    /// assert_eq!([0u8, 1, 2, 3, 4, 5], output);
    /// ```
    pub struct SparseReader<'a, 'b> {
        mapping_index: usize,
        position: usize,
        mappings: &'b [&'a [u8]],
    }
    impl<'a, 'b> SparseReader<'a, 'b> {
        pub fn new(mappings: &'b [&'a [u8]]) -> SparseReader<'a, 'b> {
            SparseReader {
                mapping_index: 0,
                position: 0,
                mappings,
            }
        }
    }
    impl<'a, 'b> Read for SparseReader<'a, 'b> {
        fn read(&mut self, mut buf: &mut [u8]) -> std::io::Result<usize> {
            let mut bytes_remaining = buf.len();
            let mut bytes_read = 0;
            if self.mapping_index >= self.mappings.len() {
                return Ok(0);
            }
            for (idx, mapping) in self
                .mappings
                .iter()
                .enumerate()
                .skip(self.mapping_index)
            {
                let (mapping_start, mapping_len) = if idx == self.mapping_index {
                    (self.position, mapping.len() - self.position)
                } else {
                    (0, mapping.len())
                };
                let bytes_to_copy = std::cmp::min(bytes_remaining, mapping_len);
                buf[..bytes_to_copy]
                    .copy_from_slice(
                        &mapping[mapping_start..(mapping_start + bytes_to_copy)],
                    );
                buf = &mut buf[bytes_to_copy..];
                bytes_read += bytes_to_copy;
                bytes_remaining -= bytes_to_copy;
                if bytes_remaining == 0
                    || (idx == self.mappings.len() - 1
                        && mapping_start + bytes_to_copy == mapping.len())
                {
                    self.mapping_index = idx;
                    self.position = mapping_start + bytes_to_copy;
                    if self.position == self.mappings[self.mapping_index].len() {
                        self.mapping_index += 1;
                        self.position = 0;
                    }
                    break;
                }
            }
            Ok(bytes_read)
        }
    }
}
mod util {
    use binrw::NullString;
    use binrw::NullWideString;
    use chrono::prelude::*;
    use chrono::Duration;
    use serde::Serializer;
    use crate::StfTimestamp;
    pub fn serialize_null_string<S>(x: &NullString, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        s.serialize_str(x.to_string().as_str())
    }
    pub fn serialize_null_wide_string<S>(
        x: &NullWideString,
        s: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        s.serialize_str(x.to_string().as_str())
    }
    pub fn stf_timestamp_to_chrono(timestamp: StfTimestamp) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(
                1980 + (timestamp.year() as i32),
                timestamp.month() as u32,
                timestamp.day() as u32,
                timestamp.hour() as u32,
                timestamp.minute() as u32,
                (timestamp.seconds() as u32) << 1,
            )
            .unwrap()
    }
    pub fn windows_filetime_to_chrono(high: u32, low: u32) -> DateTime<Utc> {
        let time_as_i64 = (((high as u64) << 32) | low as u64) as i64;
        Utc.with_ymd_and_hms(1601, 1, 1, 0, 0, 0).unwrap()
            + Duration::nanoseconds(time_as_i64 * 100)
    }
}
pub use crate::parse::*;
pub use binrw;
pub use vfs;
