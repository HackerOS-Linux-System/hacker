# hacker

Parser for `.hacker` files — written in pure **H#**, for [bit](https://github.com/HackerOS-Linux-System/bit).

Earlier versions were thin H# bindings over a Rust crate (`hacker_parser`, linked statically through
`extern static [rust, ...]`). This version is a full rewrite in H#: no Rust toolchain, no FFI, no
handles to free — `parse` just returns a value.

## Install

```
[dependencies]
-> hacker => newest
```

## File formats

A `.hacker` file is everything between one pair of square brackets. The version is detected
automatically, trying v2, then v3, then v1 (v1 accepts anything).

```
! v1 — free text
[
  your config
]

! v2 — header, then `name:` sections with `= value` entries
[
  Header
  Section:
  = description
  = another value
]

! v3 — exactly one of each field, in any order
[
  = header
  => type
  -> description
  --> file
]
```

Rules worth knowing:

* Lines are trimmed and empty lines are ignored.
* v2: the header is every line before the first line ending in `:`. A section exists only once it has
  at least one `= value`; a section name that appears twice is merged into one.
* v3: all four fields are required and none may repeat.
* Anything that is not valid v2 or v3 falls back to v1, so the only real errors are
  `MissingBrackets` and `EmptyContent`.

## Usage

```hsharp
use "bit -> hacker"
use "std -> option" from "option"

fn main() is
    match hacker::parse_file("app.hacker") is
        option::Result::Ok(f) => is
            if hacker::is_v3(f) is
                write(hacker::v3_header(f) + " (" + hacker::v3_type(f) + ")")
            end
        end
        option::Result::Err(e) => is
            write(hacker::error_message(e))
        end
    end
end
```

A complete example is in [`examples/parse.h#`](examples/parse.h#).

## API

Types: `HackerFile` (`V1(HackerV1)` / `V2(HackerV2)` / `V3(HackerV3)`), `HackerV1 { content }`,
`HackerV2 { header, sections: [Section] }`, `Section { name, values }`,
`HackerV3 { header, ty, description, file }`, and the error enum `ParseError`.

| function | |
|---|---|
| `parse(input) -> Result<HackerFile, ParseError>` | parse a string |
| `parse_file(path) -> Result<HackerFile, ParseError>` | read and parse a file (`IoError` if it cannot be read) |
| `error_message(e) -> string` | text of an error (the same Polish messages as the Rust crate) |
| `version(f)`, `is_v1(f)`, `is_v2(f)`, `is_v3(f)` | which version was parsed (1, 2 or 3) |
| `v1_content(f)` | v1 text |
| `v2_header(f)`, `v2_sections(f)` | v2 header / all sections |
| `v2_section_count(f)`, `v2_section_names(f)`, `v2_section_name(f, i)` | section names, sorted alphabetically |
| `v2_has_section(f, name)`, `v2_values(f, name)`, `v2_value_count(f, name)`, `v2_value_at(f, name, i)` | section values in file order |
| `v3_header(f)`, `v3_type(f)`, `v3_description(f)`, `v3_file(f)` | v3 fields |

Accessors for a different version return `""`, `0` or an empty array instead of failing.

## Migrating from the Rust-backed binding

| before | now |
|---|---|
| `let h: int = hacker::parse(s)` and `hacker::is_valid(h)` | `match hacker::parse(s) is option::Result::Ok(f) => ... option::Result::Err(e) => ... end` |
| `hacker::free(h)`, `hacker::free_string(x)` | nothing to free |
| `hacker::v2_value_at(h, name, i)` etc. | same names, first argument is the `HackerFile` |
| `hacker::version_v1()` … | compare `hacker::version(f)` with `1`, `2`, `3`, or use `is_v1/2/3` |
| a parse error was the handle `0` | `ParseError` with a message |

## Differences from the Rust crate

* `HackerV2.sections` is an ordered list of `Section` instead of a `HashMap` (Rust's iteration order was
  unspecified). Lookups by name behave the same; `v2_section_names` is sorted, as before.
* `IoError` carries a fixed message (`nie można odczytać pliku: <path>`) instead of the OS error text.
* No `Debug` / `Clone` / `PartialEq` derives — compare the fields you care about.

Parsing itself is identical to the Rust implementation, including trimming of every Unicode whitespace
character and `\r\n` line endings.

## Development

```
bit check     # type-check
bit build     # build the library
bit test      # run the #[test] functions in src/lib.h#
```

## License

MIT
