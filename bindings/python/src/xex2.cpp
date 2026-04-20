// Nanobind glue over the diplomat-generated C++ wrappers for `xex2-ffi`.
// The `crates/xex2-ffi/bindings/cpp/*.hpp` files are fully auto-generated;
// this file is the minimal hand-written layer mapping diplomat's
// `std::unique_ptr<T>` / `diplomat::result<T, Xex2Error>` idioms onto
// Python classes and exceptions.
//
// All method patterns are mechanical:
//   - `Result<T, Error>` → unwrap or throw RuntimeError with message
//   - `unique_ptr<Collection>` (imports / resources) → wrap in PyHandle
//   - `copy_<field>(span)` fixed-size byte accessors → return `bytes`
//   - `Xex2Bytes` owned-buffer results → return `bytes`
//
// Regenerate the diplomat headers after any change to
// `crates/xex2-ffi/src/lib.rs`:
//
//   diplomat-tool cpp crates/xex2-ffi/bindings/cpp \
//       --entry crates/xex2-ffi/src/lib.rs

#include <nanobind/nanobind.h>
#include <nanobind/stl/string.h>

#include <cstddef>
#include <cstdint>
#include <stdexcept>
#include <string>
#include <utility>

#include "Xex2.hpp"
#include "Xex2Bytes.hpp"
#include "Xex2Error.hpp"
#include "Xex2ImportLibrary.hpp"
#include "Xex2Imports.hpp"
#include "Xex2RemoveLimits.hpp"
#include "Xex2Resource.hpp"
#include "Xex2Resources.hpp"

namespace nb = nanobind;

// ──────────────────────────────────────────────────────────────────────────
// Result / bytes helpers
// ──────────────────────────────────────────────────────────────────────────

namespace {

// Turn a diplomat Xex2Error into std::runtime_error (nanobind converts
// to Python RuntimeError).
[[noreturn]] void throw_xex2_error(std::unique_ptr<::Xex2Error> err) {
	throw std::runtime_error(err->message());
}

// Unwrap diplomat::result<T, Xex2Error>. On error, throw. This is the
// single chokepoint for error propagation from Rust through to Python.
template <typename T>
T unwrap(diplomat::result<T, std::unique_ptr<::Xex2Error>>&& r) {
	if (!r.is_ok()) throw_xex2_error(std::move(r).err().value());
	return std::move(r).ok().value();
}

// Wrapper around a Xex2Bytes handle: one shot to copy the owned byte
// buffer into a Python `bytes` object.
nb::bytes bytes_from_xex2_bytes(std::unique_ptr<::Xex2Bytes> buf) {
	size_t len = buf->len();
	std::string tmp(len, '\0');
	buf->copy_into(diplomat::span<uint8_t>(reinterpret_cast<uint8_t*>(tmp.data()), len));
	return nb::bytes(tmp.data(), len);
}

// Pump a `copy_<field>(span)` accessor into a fixed-size `bytes`.
template <typename Fn>
nb::bytes bytes_from_copy(Fn&& copy_fn, size_t size) {
	std::string tmp(size, '\0');
	size_t n = copy_fn(diplomat::span<uint8_t>(reinterpret_cast<uint8_t*>(tmp.data()), size));
	return nb::bytes(tmp.data(), n);
}

// Python-side wrapper structs. Plain structs hold a unique_ptr<T> so
// nanobind can freely move/default-construct; member lambdas access the
// underlying diplomat handle through `.inner`.
struct PyXex2 { std::unique_ptr<::Xex2> inner; };
struct PyXex2Imports { std::unique_ptr<::Xex2Imports> inner; };
struct PyXex2ImportLibrary { std::unique_ptr<::Xex2ImportLibrary> inner; };
struct PyXex2Resources { std::unique_ptr<::Xex2Resources> inner; };
struct PyXex2Resource { std::unique_ptr<::Xex2Resource> inner; };
struct PyXex2RemoveLimits { std::unique_ptr<::Xex2RemoveLimits> inner; };

} // namespace

// ──────────────────────────────────────────────────────────────────────────
// Module
// ──────────────────────────────────────────────────────────────────────────

NB_MODULE(xex2, m) {
	m.doc() =
		"Python bindings for the xex2 crate. Header/image/exec fields, "
		"imports, resources, basefile extraction, and restriction-removal "
		"modify are all exposed; absent optional fields raise RuntimeError "
		"with a specific message (propagated from the Rust side).";

	// ── Xex2RemoveLimits ───────────────────────────────────────────────
	nb::class_<PyXex2RemoveLimits>(m, "Xex2RemoveLimits")
		.def(
			"__init__",
			[](PyXex2RemoveLimits* self) { new (self) PyXex2RemoveLimits{::Xex2RemoveLimits::new_()}; },
			"Construct empty (all flags false).")
		.def_static(
			"all",
			[]() { return PyXex2RemoveLimits{::Xex2RemoveLimits::all()}; },
			"All flags true — strip every known restriction.")
		.def("set_media", [](PyXex2RemoveLimits& s, bool v) { s.inner->set_media(v); })
		.def("set_region", [](PyXex2RemoveLimits& s, bool v) { s.inner->set_region(v); })
		.def("set_bounding_path", [](PyXex2RemoveLimits& s, bool v) { s.inner->set_bounding_path(v); })
		.def("set_device_id", [](PyXex2RemoveLimits& s, bool v) { s.inner->set_device_id(v); })
		.def("set_console_id", [](PyXex2RemoveLimits& s, bool v) { s.inner->set_console_id(v); })
		.def("set_dates", [](PyXex2RemoveLimits& s, bool v) { s.inner->set_dates(v); })
		.def("set_keyvault_privileges", [](PyXex2RemoveLimits& s, bool v) { s.inner->set_keyvault_privileges(v); })
		.def("set_signed_keyvault_only", [](PyXex2RemoveLimits& s, bool v) { s.inner->set_signed_keyvault_only(v); })
		.def("set_library_versions", [](PyXex2RemoveLimits& s, bool v) { s.inner->set_library_versions(v); })
		.def("set_zero_media_id", [](PyXex2RemoveLimits& s, bool v) { s.inner->set_zero_media_id(v); });

	// ── Xex2ImportLibrary ──────────────────────────────────────────────
	nb::class_<PyXex2ImportLibrary>(m, "Xex2ImportLibrary")
		.def_prop_ro("name", [](const PyXex2ImportLibrary& s) { return s.inner->name(); })
		.def_prop_ro("import_id", [](const PyXex2ImportLibrary& s) { return s.inner->import_id(); })
		.def_prop_ro("version", [](const PyXex2ImportLibrary& s) { return s.inner->version(); })
		.def_prop_ro("version_min", [](const PyXex2ImportLibrary& s) { return s.inner->version_min(); })
		.def_prop_ro("record_count", [](const PyXex2ImportLibrary& s) { return s.inner->record_count(); })
		.def("__len__", [](const PyXex2ImportLibrary& s) { return s.inner->record_count(); })
		.def(
			"record_at",
			[](const PyXex2ImportLibrary& s, size_t idx) {
				return unwrap(s.inner->record_at(idx));
			},
			nb::arg("idx"))
		.def_prop_ro("digest", [](const PyXex2ImportLibrary& s) {
			return bytes_from_copy([&](auto buf) { return s.inner->copy_digest(buf); }, 20);
		});

	// ── Xex2Imports ────────────────────────────────────────────────────
	nb::class_<PyXex2Imports>(m, "Xex2Imports")
		.def("__len__", [](const PyXex2Imports& s) { return s.inner->len(); })
		.def_prop_ro("len", [](const PyXex2Imports& s) { return s.inner->len(); })
		.def(
			"get",
			[](const PyXex2Imports& s, size_t idx) {
				return PyXex2ImportLibrary{unwrap(s.inner->get(idx))};
			},
			nb::arg("idx"))
		.def("__getitem__", [](const PyXex2Imports& s, size_t idx) {
			return PyXex2ImportLibrary{unwrap(s.inner->get(idx))};
		});

	// ── Xex2Resource ───────────────────────────────────────────────────
	nb::class_<PyXex2Resource>(m, "Xex2Resource")
		.def_prop_ro("name", [](const PyXex2Resource& s) { return s.inner->name(); })
		.def_prop_ro("address", [](const PyXex2Resource& s) { return s.inner->address(); })
		.def_prop_ro("size", [](const PyXex2Resource& s) { return s.inner->size(); });

	// ── Xex2Resources ──────────────────────────────────────────────────
	nb::class_<PyXex2Resources>(m, "Xex2Resources")
		.def("__len__", [](const PyXex2Resources& s) { return s.inner->len(); })
		.def_prop_ro("len", [](const PyXex2Resources& s) { return s.inner->len(); })
		.def(
			"get",
			[](const PyXex2Resources& s, size_t idx) {
				return PyXex2Resource{unwrap(s.inner->get(idx))};
			},
			nb::arg("idx"))
		.def("__getitem__", [](const PyXex2Resources& s, size_t idx) {
			return PyXex2Resource{unwrap(s.inner->get(idx))};
		});

	// ── Xex2 ───────────────────────────────────────────────────────────
	nb::class_<PyXex2>(m, "Xex2")
		.def_static(
			"parse",
			[](nb::bytes data) {
				diplomat::span<const uint8_t> view(
					reinterpret_cast<const uint8_t*>(data.c_str()), data.size());
				return PyXex2{unwrap(::Xex2::parse(view))};
			},
			nb::arg("data"),
			"Parse an XEX2 file from bytes. RuntimeError on parse failure.")

		// SecurityInfo / ImageInfo / header — always present
		.def_prop_ro("load_address", [](const PyXex2& s) { return s.inner->load_address(); })
		.def_prop_ro("image_size", [](const PyXex2& s) { return s.inner->image_size(); })
		.def_prop_ro("header_size", [](const PyXex2& s) { return s.inner->header_size(); })
		.def_prop_ro("page_descriptor_count", [](const PyXex2& s) { return s.inner->page_descriptor_count(); })
		.def_prop_ro("info_size", [](const PyXex2& s) { return s.inner->info_size(); })
		.def_prop_ro("image_flags", [](const PyXex2& s) { return s.inner->image_flags(); })
		.def_prop_ro("import_table_count", [](const PyXex2& s) { return s.inner->import_table_count(); })
		.def_prop_ro("export_table_address", [](const PyXex2& s) { return s.inner->export_table_address(); })
		.def_prop_ro("game_regions", [](const PyXex2& s) { return s.inner->game_regions(); })
		.def_prop_ro("allowed_media_types", [](const PyXex2& s) { return s.inner->allowed_media_types(); })
		.def_prop_ro("module_flags", [](const PyXex2& s) { return s.inner->module_flags(); })
		.def_prop_ro("data_offset", [](const PyXex2& s) { return s.inner->data_offset(); })
		.def_prop_ro("security_offset", [](const PyXex2& s) { return s.inner->security_offset(); })
		.def_prop_ro("optional_header_count", [](const PyXex2& s) { return s.inner->optional_header_count(); })

		// Optional-header-backed fields — raise RuntimeError when absent
		.def_prop_ro("entry_point", [](const PyXex2& s) { return unwrap(s.inner->entry_point()); })
		.def_prop_ro("original_base_address", [](const PyXex2& s) { return unwrap(s.inner->original_base_address()); })
		.def_prop_ro("default_stack_size", [](const PyXex2& s) { return unwrap(s.inner->default_stack_size()); })
		.def_prop_ro("default_heap_size", [](const PyXex2& s) { return unwrap(s.inner->default_heap_size()); })
		.def_prop_ro("default_fs_cache_size", [](const PyXex2& s) { return unwrap(s.inner->default_fs_cache_size()); })
		.def_prop_ro("date_range_not_before", [](const PyXex2& s) { return unwrap(s.inner->date_range_not_before()); })
		.def_prop_ro("date_range_not_after", [](const PyXex2& s) { return unwrap(s.inner->date_range_not_after()); })
		.def_prop_ro("bounding_path", [](const PyXex2& s) { return unwrap(s.inner->bounding_path()); })

		// ExecutionInfo
		.def_prop_ro("title_id", [](const PyXex2& s) { return unwrap(s.inner->title_id()); })
		.def_prop_ro("exec_media_id", [](const PyXex2& s) { return unwrap(s.inner->exec_media_id()); })
		.def_prop_ro("version", [](const PyXex2& s) { return unwrap(s.inner->version()); })
		.def_prop_ro("base_version", [](const PyXex2& s) { return unwrap(s.inner->base_version()); })
		.def_prop_ro("platform", [](const PyXex2& s) { return unwrap(s.inner->platform()); })
		.def_prop_ro("executable_table", [](const PyXex2& s) { return unwrap(s.inner->executable_table()); })
		.def_prop_ro("disc_number", [](const PyXex2& s) { return unwrap(s.inner->disc_number()); })
		.def_prop_ro("disc_count", [](const PyXex2& s) { return unwrap(s.inner->disc_count()); })
		.def_prop_ro("savegame_id", [](const PyXex2& s) { return unwrap(s.inner->savegame_id()); })

		// FileFormatInfo
		.def_prop_ro("compression_type", [](const PyXex2& s) { return unwrap(s.inner->compression_type()); })
		.def_prop_ro("encryption_type", [](const PyXex2& s) { return unwrap(s.inner->encryption_type()); })
		.def_prop_ro("window_size", [](const PyXex2& s) { return unwrap(s.inner->window_size()); })

		// Fixed-size byte fields
		.def_prop_ro("image_hash", [](const PyXex2& s) {
			return bytes_from_copy([&](auto buf) { return s.inner->copy_image_hash(buf); }, 20);
		})
		.def_prop_ro("import_table_hash", [](const PyXex2& s) {
			return bytes_from_copy([&](auto buf) { return s.inner->copy_import_table_hash(buf); }, 20);
		})
		.def_prop_ro("header_hash", [](const PyXex2& s) {
			return bytes_from_copy([&](auto buf) { return s.inner->copy_header_hash(buf); }, 20);
		})
		.def_prop_ro("media_id", [](const PyXex2& s) {
			return bytes_from_copy([&](auto buf) { return s.inner->copy_media_id(buf); }, 16);
		})
		.def_prop_ro("file_key", [](const PyXex2& s) {
			return bytes_from_copy([&](auto buf) { return s.inner->copy_file_key(buf); }, 16);
		})
		.def_prop_ro("rsa_signature", [](const PyXex2& s) {
			return bytes_from_copy([&](auto buf) { return s.inner->copy_rsa_signature(buf); }, 256);
		})

		// Operations
		.def(
			"extract_basefile",
			[](const PyXex2& s) { return bytes_from_xex2_bytes(unwrap(s.inner->extract_basefile())); },
			"Decrypt and decompress the inner PE image.")
		.def(
			"modify",
			[](const PyXex2& s, const PyXex2RemoveLimits& limits) {
				return bytes_from_xex2_bytes(unwrap(s.inner->modify(*limits.inner)));
			},
			nb::arg("limits"),
			"Apply restriction-removal limits and return the re-signed XEX bytes.")

		// Collections
		.def("imports", [](const PyXex2& s) { return PyXex2Imports{s.inner->imports()}; })
		.def("resources", [](const PyXex2& s) { return PyXex2Resources{s.inner->resources()}; });
}
