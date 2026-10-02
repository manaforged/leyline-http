#![no_main]

use std::hint::black_box;

use leyline::html::{self, Form};
use libfuzzer_sys::fuzz_target;

const META_NAMES: [&str; 2] = ["csrf-token", "og:title"];

fuzz_target!(|data: &[u8]| {
    let document = String::from_utf8_lossy(data);
    for mut form in html::forms(&document) {
        let names: Vec<String> = form.fields().iter().map(|(name, _)| name.clone()).collect();
        for name in names {
            form.set(name, "x");
        }
        let buttons: Vec<String> = form.buttons().into_iter().map(str::to_owned).collect();
        for button in buttons {
            black_box(form.press(&button));
        }
        black_box(form);
    }
    black_box(Form::find(&document, "login"));
    black_box(html::links(&document));
    for name in META_NAMES {
        black_box(html::meta(&document, name));
    }
});
