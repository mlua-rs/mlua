use std::collections::HashMap;
use std::hash::Hash;
use std::mem;
use std::os::raw::c_char;

use crate::state::ExtraData;

use super::json::{self, Json};

/// Represents a heap dump of a Luau memory state.
#[cfg(any(feature = "luau", doc))]
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
pub struct HeapDump {
    data: Json<'static>, // refers to the contents of `buf`
    buf: Box<str>,
}

impl HeapDump {
    /// Dumps the current Lua heap state.
    pub(crate) unsafe fn new(state: *mut ffi::lua_State) -> Option<Self> {
        unsafe extern "C" fn category_name(state: *mut ffi::lua_State, cat: u8) -> *const c_char {
            (&*ExtraData::get(state))
                .mem_categories
                .get(cat as usize)
                .map(|s| s.as_ptr())
                .unwrap_or(cstr!("unknown"))
        }

        let mut buf = Vec::new();
        unsafe {
            let file = libc::tmpfile();
            if file.is_null() {
                return None;
            }
            ffi::lua_gcdump(state, file as *mut _, Some(category_name));
            libc::fseek(file, 0, libc::SEEK_END);
            let len = libc::ftell(file) as usize;
            libc::rewind(file);
            if len > 0 {
                buf.reserve(len);
                libc::fread(buf.as_mut_ptr() as *mut _, 1, len, file);
                buf.set_len(len);
            }
            libc::fclose(file);
        }

        let buf = String::from_utf8(buf).ok()?.into_boxed_str();
        let data = json::parse(unsafe { mem::transmute::<&str, &'static str>(&buf) }).ok()?;
        Some(HeapDump { data, buf })
    }

    /// Returns the raw JSON representation of the heap dump.
    ///
    /// The JSON structure is an internal detail and may change in future versions.
    #[doc(hidden)]
    pub fn to_json(&self) -> &str {
        &self.buf
    }

    /// Returns the total size of the Lua heap in bytes.
    pub fn size(&self) -> u64 {
        self.data["stats"]["size"].as_u64().unwrap_or_default()
    }

    /// Returns a mapping from object type to (count, total size in bytes).
    ///
    /// If `category` is provided, only objects in that category are considered.
    pub fn size_by_type<'a>(&'a self, category: Option<&str>) -> HashMap<&'a str, (usize, u64)> {
        self.size_by_type_inner(category).unwrap_or_default()
    }

    fn size_by_type_inner<'a>(&'a self, category: Option<&str>) -> Option<HashMap<&'a str, (usize, u64)>> {
        let category_id = match category {
            // If we cannot find the category, return empty result
            Some(cat) => Some(self.find_category_id(cat)?),
            None => None,
        };

        let mut size_by_type = HashMap::new();
        let objects = self.data["objects"].as_object()?;
        for obj in objects.values() {
            if let Some(cat_id) = category_id {
                if obj["cat"].as_i64()? != cat_id {
                    continue;
                }
            }
            update_size(&mut size_by_type, obj["type"].as_str()?, obj["size"].as_u64()?);
        }
        Some(size_by_type)
    }

    /// Returns a mapping from category name to total size in bytes.
    pub fn size_by_category(&self) -> HashMap<&str, u64> {
        let mut size_by_category = HashMap::new();
        if let Some(categories) = self.data["stats"]["categories"].as_object() {
            for cat in categories.values() {
                if let Some(cat_name) = cat["name"].as_str() {
                    size_by_category.insert(cat_name, cat["size"].as_u64().unwrap_or_default());
                }
            }
        }
        size_by_category
    }

    /// Returns a mapping from userdata type to (count, total size in bytes).
    pub fn size_by_userdata<'a>(&'a self, category: Option<&str>) -> HashMap<&'a str, (usize, u64)> {
        self.size_by_userdata_inner(category).unwrap_or_default()
    }

    fn size_by_userdata_inner<'a>(
        &'a self,
        category: Option<&str>,
    ) -> Option<HashMap<&'a str, (usize, u64)>> {
        let category_id = match category {
            // If we cannot find the category, return empty result
            Some(cat) => Some(self.find_category_id(cat)?),
            None => None,
        };

        let mut size_by_userdata = HashMap::new();
        let objects = self.data["objects"].as_object()?;
        for obj in objects.values() {
            if obj["type"] != "userdata" {
                continue;
            }
            if let Some(cat_id) = category_id {
                if obj["cat"].as_i64()? != cat_id {
                    continue;
                }
            }

            // Determine userdata type from metatable
            let mut ud_type = "unknown";
            if let Some(metatable_addr) = obj["metatable"].as_str() {
                if let Some(t) = get_key(objects, &objects[metatable_addr], "__type") {
                    ud_type = t;
                }
            }
            update_size(&mut size_by_userdata, ud_type, obj["size"].as_u64()?);
        }
        Some(size_by_userdata)
    }

    /// Finds the category ID for a given category name.
    fn find_category_id(&self, category: &str) -> Option<i64> {
        let categories = self.data["stats"]["categories"].as_object()?;
        for (cat_id, cat) in categories {
            if cat["name"].as_str() == Some(category) {
                return cat_id.parse().ok();
            }
        }
        None
    }
}

/// Updates the size mapping for a given key.
fn update_size<K: Eq + Hash>(size_type: &mut HashMap<K, (usize, u64)>, key: K, size: u64) {
    let (ref mut count, ref mut total_size) = size_type.entry(key).or_insert((0, 0));
    *count += 1;
    *total_size += size;
}

/// Retrieves the value associated with a given `key` from a Lua table `tbl`.
fn get_key<'a>(objects: &'a HashMap<&'a str, Json>, tbl: &Json, key: &str) -> Option<&'a str> {
    let pairs = tbl["pairs"].as_array()?;
    for kv in pairs.chunks_exact(2) {
        #[rustfmt::skip]
        let (Some(key_addr), Some(val_addr)) = (kv[0].as_str(), kv[1].as_str()) else { continue; };
        if objects[key_addr]["type"] == "string" && objects[key_addr]["data"].as_str() == Some(key) {
            if objects[val_addr]["type"] == "string" {
                return objects[val_addr]["data"].as_str();
            } else {
                break;
            }
        }
    }
    None
}
