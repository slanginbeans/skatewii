//! Skate 3's compiled state graphs (`.stategraph`), read only far enough to list every attribute value.
//!
//! Layout from SK8-ENGINE/skate-3-rust-engine crates/skate-data/src/state_graph.rs (GPL-3.0): an element is
//! a NUL-terminated tag, a big-endian attribute count, attributes (name and text NUL-terminated, a float
//! word, a boolean byte), a big-endian child count, then its children in order. Includes and template
//! parameters are already expanded on the disc.

#[allow(dead_code)] // tag and name: for format research and tests
pub struct Attr<'a> {
    /// The `name` values of the enclosing elements (and this one), outermost first, joined with `/`.
    pub path: std::rc::Rc<str>,
    pub tag: &'a str,
    pub name: &'a str,
    pub text: &'a str,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn string(&mut self) -> Result<&'a str, String> {
        let rest = self.bytes.get(self.at..).ok_or("truncated")?;
        let len = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| format!("unterminated string at {:#x}", self.at))?;
        let s = std::str::from_utf8(&rest[..len])
            .map_err(|_| format!("non-UTF-8 string at {:#x}", self.at))?;
        self.at += len + 1;
        Ok(s)
    }
    fn word(&mut self) -> Result<u32, String> {
        let b = self
            .bytes
            .get(self.at..self.at + 4)
            .ok_or_else(|| format!("truncated at {:#x}", self.at))?;
        self.at += 4;
        Ok(u32::from_be_bytes(b.try_into().unwrap()))
    }
}

/// Every attribute of every element, in file order.
pub fn attributes(bytes: &[u8]) -> Result<Vec<Attr<'_>>, String> {
    let mut r = Reader { bytes, at: 0 };
    let mut out = Vec::new();
    // Children still to read at each open level, and that level's path.
    let mut stack: Vec<(u32, std::rc::Rc<str>)> = vec![(1, "".into())];
    while let Some((remaining, parent)) = stack.last_mut() {
        if *remaining == 0 {
            stack.pop();
            continue;
        }
        *remaining -= 1;
        let parent = parent.clone();
        let tag = r.string()?;
        let count = r.word()?;
        if count as usize > (bytes.len() - r.at) / 7 {
            return Err("attribute count exceeds the data".into());
        }
        let first = out.len();
        let mut path = parent.clone();
        for _ in 0..count {
            let name = r.string()?;
            let text = r.string()?;
            r.word()?;
            r.at += 1;
            if name == "name" {
                path = if parent.is_empty() {
                    text.into()
                } else {
                    format!("{parent}/{text}").into()
                };
            }
            out.push(Attr {
                path: parent.clone(),
                tag,
                name,
                text,
            });
        }
        for a in &mut out[first..] {
            a.path = path.clone();
        }
        let children = r.word()?;
        if children as usize > bytes.len().saturating_sub(r.at) / 9 {
            return Err("child count exceeds the data".into());
        }
        stack.push((children, path));
    }
    if r.at != bytes.len() {
        return Err("trailing bytes after the root element".into());
    }
    Ok(out)
}

/// The whole element tree, with each attribute's float word and boolean byte (for the graph compiler,
/// `game_core::graph::write`).
pub fn elements(bytes: &[u8]) -> Result<game_core::graph::write::Element, String> {
    use game_core::graph::write::{AttrIn, Element};
    fn read(r: &mut Reader, depth: usize) -> Result<Element, String> {
        if depth > 64 {
            return Err("elements nested too deeply".into());
        }
        let tag = r.string()?.to_string();
        let count = r.word()?;
        if count as usize > (r.bytes.len() - r.at) / 7 {
            return Err("attribute count exceeds the data".into());
        }
        let mut attrs = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let name = r.string()?.to_string();
            let text = r.string()?.to_string();
            let float = r.word()?;
            let flag = *r.bytes.get(r.at).ok_or("truncated")?;
            r.at += 1;
            attrs.push(AttrIn {
                name,
                text,
                float,
                flag,
            });
        }
        let n = r.word()?;
        if n as usize > r.bytes.len().saturating_sub(r.at) / 9 {
            return Err("child count exceeds the data".into());
        }
        let mut children = Vec::with_capacity(n as usize);
        for _ in 0..n {
            children.push(read(r, depth + 1)?);
        }
        Ok(Element {
            tag,
            attrs,
            children,
        })
    }
    let mut r = Reader { bytes, at: 0 };
    let e = read(&mut r, 0)?;
    if r.at != bytes.len() {
        return Err("trailing bytes after the root element".into());
    }
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(out: &mut Vec<u8>, tag: &str, attrs: &[(&str, &str)], children: u32) {
        out.extend_from_slice(tag.as_bytes());
        out.push(0);
        out.extend_from_slice(&(attrs.len() as u32).to_be_bytes());
        for (n, t) in attrs {
            out.extend_from_slice(n.as_bytes());
            out.push(0);
            out.extend_from_slice(t.as_bytes());
            out.push(0);
            out.extend_from_slice(&0f32.to_be_bytes());
            out.push(0);
        }
        out.extend_from_slice(&children.to_be_bytes());
    }

    #[test]
    fn reads_nested_elements_in_order() {
        let mut b = Vec::new();
        element(&mut b, "state", &[("name", "Top")], 2);
        element(&mut b, "anim", &[("anim", "CLIP_A")], 1);
        element(&mut b, "x", &[], 0);
        element(&mut b, "anim", &[("anim", "TREE_B"), ("speed", "1")], 0);
        let a = attributes(&b).unwrap();
        let texts: Vec<&str> = a.iter().map(|a| a.text).collect();
        assert_eq!(texts, ["Top", "CLIP_A", "TREE_B", "1"]);
        assert_eq!((a[2].tag, a[2].name), ("anim", "anim"));
        assert_eq!(&*a[2].path, "Top");
        b.push(0);
        assert!(attributes(&b).is_err(), "trailing bytes");
        assert!(attributes(&b[..10]).is_err(), "truncated");
        b.pop();
        let e = elements(&b).unwrap();
        assert_eq!(e.tag, "state");
        assert_eq!(e.children.len(), 2);
        assert_eq!(e.children[0].children[0].tag, "x");
        assert_eq!(e.children[1].attrs[1].text, "1");
    }
}
