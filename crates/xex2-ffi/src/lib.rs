//! C/C++ FFI for the `xex2` crate via [diplomat].
//!
//! Exposes most of the inspection + modification surface of `xex2` as
//! opaque handles. Optional headers and fields that `xex2` models as
//! `Option<T>` surface here as `Result<T, Box<Xex2Error>>` — callers get a
//! specific error message ("no ExecutionInfo optional header", "OOB import
//! index", etc.) rather than a silent default. Iteration uses handle
//! types (`Xex2Imports`, `Xex2Resources`) with `.len()` + `.get(idx)`.
//!
//! The generated C++ wrappers in `bindings/cpp/` are regenerated with:
//!
//! ```text
//! diplomat-tool cpp crates/xex2-ffi/bindings/cpp \
//!     --entry crates/xex2-ffi/src/lib.rs
//! ```

pub(crate) mod inner;

#[allow(clippy::needless_lifetimes)]
#[diplomat::bridge]
mod ffi {
	use diplomat_runtime::DiplomatWrite;
	use std::fmt::Write;
	use std::sync::Arc;

	// ──────────────────────────────────────────────────────────────────
	// Error + byte-buffer handles
	// ──────────────────────────────────────────────────────────────────

	/// Error returned by any fallible FFI entry point. Carries a
	/// human-readable message accessible via [`Xex2Error::message`].
	#[diplomat::opaque]
	pub struct Xex2Error(pub(crate) String);

	impl Xex2Error {
		/// Write the underlying error message into `out`.
		pub fn message(&self, out: &mut DiplomatWrite) {
			let _ = write!(out, "{}", self.0);
		}
	}

	/// Owned byte buffer — used as the return type for operations that
	/// produce arbitrary-sized output (extract / modify). Diplomat's C++
	/// backend doesn't yet return `Vec<u8>` as a native type, so we wrap
	/// it as an opaque with `.len()` + `.copy_into(buf)`.
	#[diplomat::opaque]
	pub struct Xex2Bytes(pub(crate) Vec<u8>);

	impl Xex2Bytes {
		/// Number of bytes in the buffer.
		pub fn len(&self) -> usize {
			self.0.len()
		}

		/// Copy up to `dst.len()` bytes from the buffer into `dst`. Returns
		/// the number of bytes actually written. Callers are expected to
		/// size `dst` via a prior `.len()` call.
		pub fn copy_into(&self, dst: &mut [u8]) -> usize {
			let n = self.0.len().min(dst.len());
			dst[..n].copy_from_slice(&self.0[..n]);
			n
		}
	}

	// ──────────────────────────────────────────────────────────────────
	// Xex2 — the primary handle
	// ──────────────────────────────────────────────────────────────────

	/// Parsed XEX2 file. Owns an internal copy of the input bytes so
	/// extract/modify calls don't require the caller to keep the original
	/// buffer alive.
	#[diplomat::opaque]
	pub struct Xex2(pub(crate) crate::inner::Xex2Inner);

	impl Xex2 {
		/// Parse an XEX2 file from its raw bytes.
		#[diplomat::attr(auto, named_constructor = "parse")]
		pub fn parse(bytes: &[u8]) -> Result<Box<Xex2>, Box<Xex2Error>> {
			crate::inner::parse(bytes).map(|inner| Box::new(Xex2(inner))).map_err(|msg| Box::new(Xex2Error(msg)))
		}

		// ── SecurityInfo fields (always present) ───────────────────────

		/// Virtual address where the PE image is loaded.
		pub fn load_address(&self) -> u32 {
			self.0.parsed.security_info.image_info.load_address.0
		}

		/// Size of the PE image in memory, in bytes.
		pub fn image_size(&self) -> u32 {
			self.0.parsed.security_info.image_size
		}

		/// Size of the XEX2 header region.
		pub fn header_size(&self) -> u32 {
			self.0.parsed.security_info.header_size
		}

		/// Number of page descriptors following the SecurityInfo.
		pub fn page_descriptor_count(&self) -> u32 {
			self.0.parsed.security_info.page_descriptor_count
		}

		// ── ImageInfo fields (always present) ──────────────────────────

		/// Size of the ImageInfo block within SecurityInfo.
		pub fn info_size(&self) -> u32 {
			self.0.parsed.security_info.image_info.info_size
		}

		/// Bitflags: `ImageFlags` (SMALL_PAGES, KV_PRIVILEGES_REQUIRED,
		/// SIGNED_KEYVAULT_REQUIRED).
		pub fn image_flags(&self) -> u32 {
			self.0.parsed.security_info.image_info.image_flags.bits()
		}

		/// Number of import tables.
		pub fn import_table_count(&self) -> u32 {
			self.0.parsed.security_info.image_info.import_table_count
		}

		/// Virtual address of the PE export directory, or 0 if unset.
		pub fn export_table_address(&self) -> u32 {
			self.0.parsed.security_info.image_info.export_table_address
		}

		/// Bitflags: region mask.
		pub fn game_regions(&self) -> u32 {
			self.0.parsed.security_info.image_info.game_regions
		}

		/// Bitflags: `AllowedMediaTypes`.
		pub fn allowed_media_types(&self) -> u32 {
			self.0.parsed.security_info.image_info.allowed_media_types.bits()
		}

		// ── Xex2Header fields (always present) ─────────────────────────

		/// Bitflags: `ModuleFlags` (TITLE, EXPORTS_TO_TITLE, DLL, PATCH, ...).
		pub fn module_flags(&self) -> u32 {
			self.0.parsed.header.module_flags.bits()
		}

		/// Offset to the PE data region in the XEX file.
		pub fn data_offset(&self) -> u32 {
			self.0.parsed.header.data_offset
		}

		/// Offset to the SecurityInfo region in the XEX file.
		pub fn security_offset(&self) -> u32 {
			self.0.parsed.header.security_offset
		}

		/// Number of optional header entries.
		pub fn optional_header_count(&self) -> u32 {
			self.0.parsed.header.optional_header_count
		}

		// ── Optional-header-backed Xex2Header accessors ────────────────
		// All may legitimately be absent; we return Result so Python sees
		// a typed error instead of a silent default.

		/// Entry-point virtual address from the `EntryPoint` optional
		/// header.
		pub fn entry_point(&self) -> Result<u32, Box<Xex2Error>> {
			self.0
				.parsed
				.header
				.entry_point()
				.ok_or_else(|| Box::new(Xex2Error("no EntryPoint optional header".into())))
		}

		/// `OriginalBaseAddress` optional header, if present.
		pub fn original_base_address(&self) -> Result<u32, Box<Xex2Error>> {
			self.0
				.parsed
				.header
				.original_base_address()
				.ok_or_else(|| Box::new(Xex2Error("no OriginalBaseAddress optional header".into())))
		}

		/// `DefaultStackSize` optional header, if present.
		pub fn default_stack_size(&self) -> Result<u32, Box<Xex2Error>> {
			self.0
				.parsed
				.header
				.default_stack_size()
				.ok_or_else(|| Box::new(Xex2Error("no DefaultStackSize optional header".into())))
		}

		/// `DefaultHeapSize` optional header, if present.
		pub fn default_heap_size(&self) -> Result<u32, Box<Xex2Error>> {
			self.0
				.parsed
				.header
				.get_optional_inline(xex2::header::OptionalHeaderKey::DefaultHeapSize)
				.ok_or_else(|| Box::new(Xex2Error("no DefaultHeapSize optional header".into())))
		}

		/// `DefaultFsCacheSize` optional header, if present.
		pub fn default_fs_cache_size(&self) -> Result<u32, Box<Xex2Error>> {
			self.0
				.parsed
				.header
				.get_optional_inline(xex2::header::OptionalHeaderKey::DefaultFsCacheSize)
				.ok_or_else(|| Box::new(Xex2Error("no DefaultFsCacheSize optional header".into())))
		}

		// ── DateRange optional header ──────────────────────────────────

		/// Lower bound of the execution date restriction, as a FILETIME
		/// value. Absent when the optional header isn't set or has no
		/// lower bound.
		pub fn date_range_not_before(&self) -> Result<u64, Box<Xex2Error>> {
			self.0
				.parsed
				.header
				.date_range()
				.and_then(|d| d.not_before)
				.ok_or_else(|| Box::new(Xex2Error("no DateRange.not_before".into())))
		}

		/// Upper bound of the execution date restriction, FILETIME.
		pub fn date_range_not_after(&self) -> Result<u64, Box<Xex2Error>> {
			self.0
				.parsed
				.header
				.date_range()
				.and_then(|d| d.not_after)
				.ok_or_else(|| Box::new(Xex2Error("no DateRange.not_after".into())))
		}

		// ── BoundingPath optional header ───────────────────────────────

		/// Writes the `BoundingPath` optional header into `out`. Errors
		/// when the header is absent.
		pub fn bounding_path(&self, out: &mut DiplomatWrite) -> Result<(), Box<Xex2Error>> {
			let s = self
				.0
				.bounding_path
				.as_deref()
				.ok_or_else(|| Box::new(Xex2Error("no BoundingPath optional header".into())))?;
			let _ = write!(out, "{}", s);
			Ok(())
		}

		// ── ExecutionInfo optional header ──────────────────────────────

		/// `title_id` from ExecutionInfo.
		pub fn title_id(&self) -> Result<u32, Box<Xex2Error>> {
			self.0
				.exec_info
				.as_ref()
				.map(|e| e.title_id.0)
				.ok_or_else(|| Box::new(Xex2Error("no ExecutionInfo optional header".into())))
		}

		/// `media_id` from ExecutionInfo (the u32 one — distinct from the
		/// 16-byte `ImageInfo::media_id` exposed via `copy_media_id`).
		pub fn exec_media_id(&self) -> Result<u32, Box<Xex2Error>> {
			self.0
				.exec_info
				.as_ref()
				.map(|e| e.media_id.0)
				.ok_or_else(|| Box::new(Xex2Error("no ExecutionInfo optional header".into())))
		}

		/// `version` from ExecutionInfo, packed as a u32.
		pub fn version(&self) -> Result<u32, Box<Xex2Error>> {
			self.0
				.exec_info
				.as_ref()
				.map(|e| e.version.into())
				.ok_or_else(|| Box::new(Xex2Error("no ExecutionInfo optional header".into())))
		}

		/// `base_version` from ExecutionInfo, packed as a u32.
		pub fn base_version(&self) -> Result<u32, Box<Xex2Error>> {
			self.0
				.exec_info
				.as_ref()
				.map(|e| e.base_version.into())
				.ok_or_else(|| Box::new(Xex2Error("no ExecutionInfo optional header".into())))
		}

		/// `platform` from ExecutionInfo.
		pub fn platform(&self) -> Result<u8, Box<Xex2Error>> {
			self.0
				.exec_info
				.as_ref()
				.map(|e| e.platform)
				.ok_or_else(|| Box::new(Xex2Error("no ExecutionInfo optional header".into())))
		}

		/// `executable_table` from ExecutionInfo.
		pub fn executable_table(&self) -> Result<u8, Box<Xex2Error>> {
			self.0
				.exec_info
				.as_ref()
				.map(|e| e.executable_table)
				.ok_or_else(|| Box::new(Xex2Error("no ExecutionInfo optional header".into())))
		}

		/// `disc_number` from ExecutionInfo (1-based).
		pub fn disc_number(&self) -> Result<u8, Box<Xex2Error>> {
			self.0
				.exec_info
				.as_ref()
				.map(|e| e.disc_number)
				.ok_or_else(|| Box::new(Xex2Error("no ExecutionInfo optional header".into())))
		}

		/// `disc_count` from ExecutionInfo.
		pub fn disc_count(&self) -> Result<u8, Box<Xex2Error>> {
			self.0
				.exec_info
				.as_ref()
				.map(|e| e.disc_count)
				.ok_or_else(|| Box::new(Xex2Error("no ExecutionInfo optional header".into())))
		}

		/// `savegame_id` from ExecutionInfo.
		pub fn savegame_id(&self) -> Result<u32, Box<Xex2Error>> {
			self.0
				.exec_info
				.as_ref()
				.map(|e| e.savegame_id)
				.ok_or_else(|| Box::new(Xex2Error("no ExecutionInfo optional header".into())))
		}

		// ── FileFormatInfo optional header ─────────────────────────────

		/// Compression-type discriminant (0 = None, 1 = Basic,
		/// 2 = Normal, 3 = Delta).
		pub fn compression_type(&self) -> Result<u32, Box<Xex2Error>> {
			self.0
				.file_format
				.as_ref()
				.map(|f| f.compression_type as u32)
				.ok_or_else(|| Box::new(Xex2Error("no FileFormatInfo optional header".into())))
		}

		/// Encryption-type discriminant (0 = None, 1 = Normal).
		pub fn encryption_type(&self) -> Result<u32, Box<Xex2Error>> {
			self.0
				.file_format
				.as_ref()
				.map(|f| f.encryption_type as u32)
				.ok_or_else(|| Box::new(Xex2Error("no FileFormatInfo optional header".into())))
		}

		/// LZX window size, when `compression_type == Normal`.
		pub fn window_size(&self) -> Result<u32, Box<Xex2Error>> {
			self.0
				.file_format
				.as_ref()
				.ok_or_else(|| Box::new(Xex2Error("no FileFormatInfo optional header".into())))
				.and_then(|f| {
					f.window_size.ok_or_else(|| Box::new(Xex2Error("FileFormatInfo has no window_size".into())))
				})
		}

		// ── Fixed-size byte fields: copy-into-slice accessors ──────────
		// All always-present (from SecurityInfo/ImageInfo). Returned
		// length is the number of bytes actually written (= field size
		// when `dst` is big enough).

		/// 20-byte SHA-1 of the PE image contents.
		pub fn copy_image_hash(&self, dst: &mut [u8]) -> usize {
			copy(&*self.0.parsed.security_info.image_info.image_hash, dst)
		}

		/// 20-byte SHA-1 of the import table data.
		pub fn copy_import_table_hash(&self, dst: &mut [u8]) -> usize {
			copy(&*self.0.parsed.security_info.image_info.import_table_hash, dst)
		}

		/// 20-byte SHA-1 of the XEX header.
		pub fn copy_header_hash(&self, dst: &mut [u8]) -> usize {
			copy(&*self.0.parsed.security_info.image_info.header_hash, dst)
		}

		/// 16-byte media identifier from `ImageInfo` (distinct from the
		/// u32 `exec_media_id`).
		pub fn copy_media_id(&self, dst: &mut [u8]) -> usize {
			copy(&self.0.parsed.security_info.image_info.media_id, dst)
		}

		/// 16-byte AES file key (session-key material).
		pub fn copy_file_key(&self, dst: &mut [u8]) -> usize {
			copy(&*self.0.parsed.security_info.image_info.file_key, dst)
		}

		/// 256-byte RSA signature from SecurityInfo.
		pub fn copy_rsa_signature(&self, dst: &mut [u8]) -> usize {
			copy(&self.0.parsed.security_info.rsa_signature, dst)
		}

		// ── Operations ─────────────────────────────────────────────────

		/// Decrypt + decompress the inner PE image. The returned handle
		/// owns the bytes; use `.len()` + `.copy_into(buf)`.
		pub fn extract_basefile(&self) -> Result<Box<Xex2Bytes>, Box<Xex2Error>> {
			crate::inner::extract_basefile(&self.0)
				.map(|b| Box::new(Xex2Bytes(b)))
				.map_err(|msg| Box::new(Xex2Error(msg)))
		}

		/// Apply the given restriction-removals and produce a re-signed
		/// XEX. The returned handle owns the bytes.
		pub fn modify(&self, limits: &Xex2RemoveLimits) -> Result<Box<Xex2Bytes>, Box<Xex2Error>> {
			crate::inner::modify(&self.0, &limits.0)
				.map(|b| Box::new(Xex2Bytes(b)))
				.map_err(|msg| Box::new(Xex2Error(msg)))
		}

		// ── Collections ────────────────────────────────────────────────

		/// Return a handle to the import libraries. The handle is always
		/// returned (even when the `ImportLibraries` optional header is
		/// absent — in that case `.len()` will be 0).
		pub fn imports(&self) -> Box<Xex2Imports> {
			Box::new(Xex2Imports(self.0.imports.clone()))
		}

		/// Return a handle to the embedded resources. Always returned;
		/// `.len()` is 0 when no ResourceInfo optional header is present.
		pub fn resources(&self) -> Box<Xex2Resources> {
			Box::new(Xex2Resources(self.0.resources.clone()))
		}
	}

	// ──────────────────────────────────────────────────────────────────
	// Imports
	// ──────────────────────────────────────────────────────────────────

	/// Collection of import libraries referenced by the XEX.
	#[diplomat::opaque]
	pub struct Xex2Imports(pub(crate) Arc<Vec<xex2::imports::ImportLibrary>>);

	impl Xex2Imports {
		/// Number of import libraries.
		pub fn len(&self) -> usize {
			self.0.len()
		}

		/// Get a library handle by index. Errors when `idx >= len()`.
		pub fn get(&self, idx: usize) -> Result<Box<Xex2ImportLibrary>, Box<Xex2Error>> {
			if idx < self.0.len() {
				Ok(Box::new(Xex2ImportLibrary { table: self.0.clone(), idx }))
			} else {
				Err(Box::new(Xex2Error(format!("import library index {} out of range (len={})", idx, self.0.len()))))
			}
		}
	}

	/// Reference to a single import library. Cheap to hold — the
	/// underlying data is shared via `Arc` with the parent `Xex2Imports`.
	#[diplomat::opaque]
	pub struct Xex2ImportLibrary {
		table: Arc<Vec<xex2::imports::ImportLibrary>>,
		idx: usize,
	}

	impl Xex2ImportLibrary {
		/// Write the library name (e.g. "xboxkrnl.exe") into `out`.
		pub fn name(&self, out: &mut DiplomatWrite) {
			let _ = write!(out, "{}", self.lib().name);
		}

		/// Per-library import identifier.
		pub fn import_id(&self) -> u32 {
			self.lib().import_id
		}

		/// Packed library version.
		pub fn version(&self) -> u32 {
			self.lib().version.into()
		}

		/// Minimum compatible packed library version.
		pub fn version_min(&self) -> u32 {
			self.lib().version_min.into()
		}

		/// Number of imported ordinal/thunk records.
		pub fn record_count(&self) -> usize {
			self.lib().records.len()
		}

		/// Get the `idx`-th import record. Errors on OOB.
		pub fn record_at(&self, idx: usize) -> Result<u32, Box<Xex2Error>> {
			self.lib().records.get(idx).copied().ok_or_else(|| {
				Box::new(Xex2Error(format!(
					"import record index {} out of range (len={})",
					idx,
					self.lib().records.len()
				)))
			})
		}

		/// 20-byte SHA-1 digest associated with the library import entry.
		pub fn copy_digest(&self, dst: &mut [u8]) -> usize {
			copy(&*self.lib().digest, dst)
		}
	}

	impl Xex2ImportLibrary {
		fn lib(&self) -> &xex2::imports::ImportLibrary {
			&self.table[self.idx]
		}
	}

	// ──────────────────────────────────────────────────────────────────
	// Resources
	// ──────────────────────────────────────────────────────────────────

	/// Collection of embedded PE resources (XUI, STRB, ...).
	#[diplomat::opaque]
	pub struct Xex2Resources(pub(crate) Arc<Vec<xex2::opt::ResourceEntry>>);

	impl Xex2Resources {
		pub fn len(&self) -> usize {
			self.0.len()
		}

		pub fn get(&self, idx: usize) -> Result<Box<Xex2Resource>, Box<Xex2Error>> {
			if idx < self.0.len() {
				Ok(Box::new(Xex2Resource { table: self.0.clone(), idx }))
			} else {
				Err(Box::new(Xex2Error(format!("resource index {} out of range (len={})", idx, self.0.len()))))
			}
		}
	}

	/// Reference to a single resource entry.
	#[diplomat::opaque]
	pub struct Xex2Resource {
		table: Arc<Vec<xex2::opt::ResourceEntry>>,
		idx: usize,
	}

	impl Xex2Resource {
		pub fn name(&self, out: &mut DiplomatWrite) {
			let _ = write!(out, "{}", self.entry().name);
		}

		/// Virtual address of the resource data.
		pub fn address(&self) -> u32 {
			self.entry().address
		}

		/// Size of the resource data in bytes.
		pub fn size(&self) -> u32 {
			self.entry().size
		}
	}

	impl Xex2Resource {
		fn entry(&self) -> &xex2::opt::ResourceEntry {
			&self.table[self.idx]
		}
	}

	// ──────────────────────────────────────────────────────────────────
	// Modify: restriction-removal limits
	// ──────────────────────────────────────────────────────────────────

	/// Opaque wrapper around `xex2::writer::RemoveLimits`. Constructed
	/// empty via `Xex2RemoveLimits::new()` or fully populated via
	/// `Xex2RemoveLimits::all()`; individual flags are toggled with the
	/// `set_*` methods.
	#[diplomat::opaque_mut]
	pub struct Xex2RemoveLimits(pub(crate) xex2::writer::RemoveLimits);

	impl Xex2RemoveLimits {
		/// All flags false. Use the setters to enable specific limits.
		#[diplomat::attr(auto, constructor)]
		pub fn new() -> Box<Xex2RemoveLimits> {
			Box::new(Xex2RemoveLimits(xex2::writer::RemoveLimits::default()))
		}

		/// All flags true — strip every known restriction.
		#[diplomat::attr(auto, named_constructor = "all")]
		pub fn all() -> Box<Xex2RemoveLimits> {
			Box::new(Xex2RemoveLimits(xex2::writer::RemoveLimits::all()))
		}

		/// Remove media-type restriction.
		pub fn set_media(&mut self, v: bool) {
			self.0.media = v;
		}
		/// Remove region restriction.
		pub fn set_region(&mut self, v: bool) {
			self.0.region = v;
		}
		/// Remove bounding-path restriction.
		pub fn set_bounding_path(&mut self, v: bool) {
			self.0.bounding_path = v;
		}
		/// Remove device-id restriction.
		pub fn set_device_id(&mut self, v: bool) {
			self.0.device_id = v;
		}
		/// Remove console-id restriction.
		pub fn set_console_id(&mut self, v: bool) {
			self.0.console_id = v;
		}
		/// Remove date-range restriction.
		pub fn set_dates(&mut self, v: bool) {
			self.0.dates = v;
		}
		/// Remove keyvault-privilege restriction.
		pub fn set_keyvault_privileges(&mut self, v: bool) {
			self.0.keyvault_privileges = v;
		}
		/// Remove signed-keyvault-only restriction.
		pub fn set_signed_keyvault_only(&mut self, v: bool) {
			self.0.signed_keyvault_only = v;
		}
		/// Remove minimum library-version restrictions.
		pub fn set_library_versions(&mut self, v: bool) {
			self.0.library_versions = v;
		}
		/// Zero the media id field.
		pub fn set_zero_media_id(&mut self, v: bool) {
			self.0.zero_media_id = v;
		}
	}

	// ──────────────────────────────────────────────────────────────────
	// Helpers
	// ──────────────────────────────────────────────────────────────────

	/// Shared copy-into-slice helper used by fixed-size byte accessors.
	fn copy(src: &[u8], dst: &mut [u8]) -> usize {
		let n = src.len().min(dst.len());
		dst[..n].copy_from_slice(&src[..n]);
		n
	}
}
