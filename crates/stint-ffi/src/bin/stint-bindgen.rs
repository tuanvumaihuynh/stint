//! Generates the Swift bindings from the built library, with the same UniFFI version the
//! library uses.

fn main() {
    uniffi::uniffi_bindgen_swift();
}
