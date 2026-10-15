// Copyright 2015 Brendan Zabarauskas and the gl-rs developers
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// The khronos_api 3.1.0 build script with one change for Buck2: the generated
// `include_bytes!` paths are relative to CARGO_MANIFEST_DIR as the library
// sees it when it compiles. The original writes the absolute path of the
// build script's working directory, which Buck2 moves after the script runs.
// Extensions are listed in name order so that the output does not depend on
// directory iteration order.
use std::env;
use std::fs::File;
use std::io::Write;
use std::path::*;

fn main() {
    let dest = env::var("OUT_DIR").unwrap();
    let mut file = File::create(&Path::new(&dest).join("webgl_exts.rs")).unwrap();

    let root = env::current_dir().unwrap().join("api_webgl/extensions");

    writeln!(file, "&[").unwrap();

    let mut names: Vec<String> = root
        .read_dir()
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    for ext_name in names {
        let path = root.join(&ext_name);
        if path.is_dir() && ext_name != "template" && path.join("extension.xml").is_file() {
            let relative = format!("/api_webgl/extensions/{ext_name}/extension.xml");
            writeln!(file, "&*include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {relative:?})),").unwrap();
        }
    }

    writeln!(file, "]").unwrap();
}
