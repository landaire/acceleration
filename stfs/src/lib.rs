pub mod stfs;
use std::panic;
pub use stfs::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn read_stfs_package(package_contents: &[u8]) -> JsValue {
    panic::set_hook(Box::new(console_error_panic_hook::hook));

    let stfs_package =
        StfsPackage::try_from(package_contents).expect("failed to read STFS package");
    JsValue::from_serde(&stfs_package).expect("failed to serialize STFS package")
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
