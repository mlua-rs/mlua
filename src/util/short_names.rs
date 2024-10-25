//! Mostly copied from [bevy_utils]
//!
//! [bevy_utils]: https://github.com/bevyengine/bevy/blob/main/crates/bevy_utils/src/short_names.rs

use std::any::type_name;

/// Returns a short version of a type name `T` without all module paths.
///
/// The short name of a type is its full name as returned by
/// [`std::any::type_name`], but with the prefix of all paths removed. For
/// example, the short name of `alloc::vec::Vec<core::option::Option<u32>>`
/// would be `Vec<Option<u32>>`.
pub(crate) fn short_type_name<T: ?Sized>() -> String {
    let full_name = type_name::<T>();

    // Generics result in nested paths within <..> blocks.
    // Consider "core::option::Option<alloc::string::String>".
    // To tackle this, we parse the string from left to right, collapsing as we go.
    let mut index: usize = 0;
    let end_of_string = full_name.len();
    let mut parsed_name = String::new();

    while index < end_of_string {
        let rest_of_string = full_name.get(index..end_of_string).unwrap_or_default();

        // Collapse everything up to the next special character,
        // then skip over it
        if let Some(special_character_index) =
            rest_of_string.find(|c: char| [' ', '<', '>', '(', ')', '[', ']', ',', ';'].contains(&c))
        {
            let segment_to_collapse = rest_of_string.get(0..special_character_index).unwrap_or_default();
            parsed_name += collapse_type_name(segment_to_collapse);
            // Insert the special character
            let special_character = &rest_of_string[special_character_index..=special_character_index];
            parsed_name.push_str(special_character);

            match special_character {
                ">" | ")" | "]" if rest_of_string[special_character_index + 1..].starts_with("::") => {
                    parsed_name.push_str("::");
                    // Move the index past the "::"
                    index += special_character_index + 3;
                }
                // Move the index just past the special character
                _ => index += special_character_index + 1,
            }
        } else {
            // If there are no special characters left, we're done!
            parsed_name += collapse_type_name(rest_of_string);
            index = end_of_string;
        }
    }
    parsed_name
}

#[inline(always)]
fn collapse_type_name(string: &str) -> &str {
    string.rsplit("::").next().unwrap()
}

#[cfg(test)]
mod tests {
    use super::short_type_name;
    use std::collections::HashMap;

    #[test]
    fn tests() {
        assert_eq!(short_type_name::<String>(), "String");
        assert_eq!(short_type_name::<Option<String>>(), "Option<String>");
        assert_eq!(short_type_name::<(String, &str)>(), "(String, &str)");
        assert_eq!(short_type_name::<[i32; 3]>(), "[i32; 3]");
        assert_eq!(
            short_type_name::<HashMap<String, Option<[i32; 3]>>>(),
            "HashMap<String, Option<[i32; 3]>>"
        );
        assert_eq!(short_type_name::<dyn Fn(i32) -> i32>(), "dyn Fn(i32) -> i32");
    }
}
