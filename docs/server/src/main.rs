//! Loopback documentation server for the deliver docs site.
//!
//! The site root is the directory above this crate (the one holding
//! `index.html`). Pass an optional `host:port` argument to listen somewhere
//! other than `127.0.0.1:8743`. Standard library only, mirroring the
//! website/server in this repository.

use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::thread;
use std::time::Duration;

fn main() {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8743".to_string());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let root = root.canonicalize().unwrap_or_else(|err| {
        eprintln!("docs directory missing ({}): {err}", root.display());
        std::process::exit(1);
    });

    let listener = TcpListener::bind(&addr).unwrap_or_else(|err| {
        eprintln!("could not bind {addr}: {err}");
        std::process::exit(1);
    });
    eprintln!("serving {} at http://{addr}/", root.display());

    for conn in listener.incoming() {
        let Ok(stream) = conn else { continue };
        let root = root.clone();
        thread::spawn(move || {
            if let Err(err) = handle(stream, &root) {
                eprintln!("connection: {err}");
            }
        });
    }
}

fn handle(mut stream: TcpStream, root: &Path) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buf = [0_u8; 4096];
    let n = stream.read(&mut buf)?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let mut lines = req.lines();
    let Some(request_line) = lines.next() else {
        return write_status(
            &mut stream,
            400,
            "text/plain; charset=utf-8",
            b"bad request",
        );
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    if method != "GET" && method != "HEAD" {
        return write_status(
            &mut stream,
            405,
            "text/plain; charset=utf-8",
            b"method not allowed",
        );
    }

    let Some(path) = safe_join(root, target) else {
        return write_status(&mut stream, 404, "text/html; charset=utf-8", NOT_FOUND);
    };
    if !path.is_file() {
        return write_status(&mut stream, 404, "text/html; charset=utf-8", NOT_FOUND);
    }
    let body = fs::read(&path)?;
    let ctype = content_type(&path);
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    if method == "GET" {
        stream.write_all(&body)?;
    }
    Ok(())
}

fn safe_join(root: &Path, target: &str) -> Option<PathBuf> {
    let path = target.split('?').next().unwrap_or("/");
    let path = path.split('#').next().unwrap_or("/");
    let decoded = percent_decode(path);
    let rel = decoded.trim_start_matches('/');
    if rel.is_empty() {
        return Some(root.join("index.html"));
    }
    let mut out = root.to_path_buf();
    for part in Path::new(rel).components() {
        match part {
            Component::Normal(seg) => out.push(seg),
            Component::CurDir => {}
            _ => return None,
        }
    }
    let canon = out.canonicalize().ok()?;
    if canon.starts_with(root) {
        Some(canon)
    } else {
        None
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("md") => "text/markdown; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn write_status(
    stream: &mut TcpStream,
    code: u16,
    ctype: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = match code {
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body)?;
    Ok(())
}

const NOT_FOUND: &[u8] = br#"<!DOCTYPE html><html lang="en"><meta charset="utf-8"><title>Not found</title><body style="font-family:Georgia,serif;background:#f3eee4;color:#1c1915;padding:2rem"><h1>Not found</h1><p><a href="/">Back to the documentation</a></p></body></html>"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        let root = env::temp_dir().join(format!("deliver-docs-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("index.html"), b"ok").unwrap();
        root.canonicalize().unwrap()
    }

    #[test]
    fn index_and_traversal() {
        let root = fixture();
        let index = safe_join(&root, "/").unwrap();
        assert_eq!(index, root.join("index.html"));
        assert!(safe_join(&root, "/index.html")
            .unwrap()
            .ends_with("index.html"));
        assert!(safe_join(&root, "/../etc/passwd").is_none());
        assert!(safe_join(&root, "/%2e%2e/etc/passwd").is_none());
        assert!(safe_join(&root, "/missing.html").is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("%2e%2e"), "..");
        assert_eq!(percent_decode("a%20b"), "a b");
    }
}
