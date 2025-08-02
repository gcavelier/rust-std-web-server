use std::{
    collections::HashMap,
    error::Error,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
};

const DEFAULT_PORT: u16 = 8080;
const DEFAULT_ADDRESS: &str = "0.0.0.0";
const DEFAULT_DIR: &str = ".";
const DEFAULT_MIME_TYPE: &str = "application/octet-stream";

struct Config {
    port: u16,
    address: String,
    directory: String,
}

#[derive(Debug)]
struct ReqInfo {
    method: String,
    path: String,
    version: String,
    headers: HashMap<String, String>,
}

fn parse_request(buf_reader: &mut BufReader<TcpStream>) -> ReqInfo {
    // This is the variable this function will return
    let mut res = ReqInfo {
        method: String::new(),
        path: String::new(),
        version: String::new(),
        headers: HashMap::new(),
    };

    let mut input = buf_reader.lines();

    if let Some(Ok(status_line)) = input.next() {
        // parse the status line
        // "GET /foo.txt HTTP/1.1"
        let mut status_iter = status_line.split(' ');
        let method = status_iter.next();
        let path = status_iter.next();
        let version = status_iter.next();
        match (method, path, version) {
            (Some(method), Some(path), Some(version)) => {
                res.method = method.to_owned();
                res.path = path.to_owned();
                res.version = version.to_owned();
            }
            _ => {
                panic!("Invalid status line: {status_line}");
            }
        };
    } else {
        panic!("Failed to get status line");
    };

    // We suppose that all the other lines are headers
    for line in input {
        let line = match line {
            Ok(line) => line,
            Err(err) => panic!("{err}"),
        };
        match line.split_once(':') {
            Some((key, value)) => res.headers.insert(key.to_owned(), value.to_owned()),
            None => break,
        };
    }

    res
}

fn url_encode(input: &str) -> String {
    let mut res = String::new();

    for c in input.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '~' | '_' | '-' => res.push(c),
            c if c.is_ascii() => res.push_str(&format!("%{:02X}", c as u8)),
            _ => unimplemented!(),
        }
    }

    res
}

fn html_encode(input: String) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn url_decode(input: &str) -> String {
    let input = input.replace("+", " ");
    let mut res = String::new();
    let mut iter = input.chars();

    while let Some(c) = iter.next() {
        if c == '%' {
            // reading 2 more characters
            let char1 = iter.next();
            let char2 = iter.next();
            match (char1, char2) {
                (Some(char1), Some(char2)) => {
                    let byte = u8::from_str_radix(&format!("{char1}{char2}"), 16).unwrap();
                    res.push(byte as char);
                }
                _ => panic!(),
            }
        } else {
            res.push(c);
        }
    }

    res
}

fn normalize_path(path: String) -> String {
    let mut res = Vec::new();

    for part in path.split("/") {
        match part {
            "" // ignore empty directories (multiple /)
          | "." => (), // ignore current directory
            ".." => {
                let _ = res.pop();
            }
            part => res.push(part),
        }
    }

    res.join("/")
}

fn list_directory(directory: &str) -> Result<String, Box<dyn Error>> {
    use std::fmt::Write;

    // This will contain HTML \o/
    let mut res = String::new();

    writeln!(
        &mut res,
        "<!DOCTYPE html>
<html lang=\"en\">
<head>
  <meta charset=\"utf-8\">
  <title>Index of {directory}</title>
  <style>
  body {{
    background-color: Canvas;
    color: CanvasText;
    color-scheme: light dark;
  }}
  a, a:visited, a:active {{
    text-decoration: none;
  }}
  </style>
</head>"
    )?;
    writeln!(&mut res, "<h1>Directory Listing</h1>")?;
    writeln!(&mut res, "<h2>Directory: {directory}</h2>")?;
    writeln!(&mut res, "<hr>")?;
    writeln!(&mut res, "<ul>")?;

    // The first entry is always '..'
    writeln!(&mut res, "  <li><a href=\"..\">..</a></li>")?;

    for path in std::fs::read_dir(directory)? {
        let path = path?;
        let path_string = path
            .file_name()
            .into_string()
            .unwrap_or_else(|_| panic!("cannot convert '{path:?}' into a string!"));
        let display_name = match path.file_type()?.is_dir() {
            true => {
                format!("📁 {path_string}/")
            }
            false => format!("📄 {path_string}"),
        };
        writeln!(
            res,
            "  <li><a href=\"{}\">{}</a></li>",
            url_encode(&path_string),
            html_encode(display_name)
        )?;
    }
    writeln!(&mut res, "</ul>")?;
    writeln!(&mut res, "<hr>")?;
    writeln!(&mut res, "</html>")?;

    Ok(res)
}

fn mime_type(file_path: &str) -> String {
    let filename = Path::new(file_path)
        .file_name()
        .unwrap_or_else(|| panic!("invalid file_path: {file_path}"));

    let Some(ext) = filename
        .to_str()
        .unwrap_or_else(|| panic!("filename '{filename:?}' is not utf-8 valid"))
        .split('.')
        .last()
    else {
        return String::from(DEFAULT_MIME_TYPE);
    };

    match ext {
        "html" | "htm" => String::from("text/html"),
        "jpeg" | "jpg" => String::from("image/jpeg"),
        "png" => String::from("image/png"),
        "txt" => String::from("text/plain"),
        "css" => String::from("text/css"),
        "js" => String::from("text/javascript"),
        "json" => String::from("application/json"),
        _ => String::from(DEFAULT_MIME_TYPE),
    }
}

fn process_request(tcp_stream: TcpStream) -> Result<(), Box<dyn Error>> {
    let mut buf_reader = BufReader::new(tcp_stream);

    let request = parse_request(&mut buf_reader);
    // validate the request
    if request.version != "HTTP/1.1" {
        panic!("unsupported HTTP version : {}", request.version);
    }
    if request.method != "GET" {
        panic!("unsupported HTTP method : {}", request.method);
    }
    if !request.path.starts_with('/') {
        panic!("path must be absolute");
    }
    println!("{} {}", request.method, request.path);

    // if we are here, we should reply to the caller
    let path = match request.path.split_once('?') {
        Some((path, _query_parameters)) => path,
        None => &request.path,
    };
    let path = url_decode(path);
    let mut path = normalize_path(path);

    // handle empty path (root path)
    if path.is_empty() {
        path.push('.');
    }

    // try to serve an index page
    let mut file = None;
    let to_try = [
        &path,
        &format!("{path}/index.html"),
        &format!("{path}/index.htm"),
    ];

    for try_ in to_try {
        if Path::new(try_).is_file() {
            file = Some(try_);
            break;
        }
    }

    let mut tcp_stream = buf_reader.into_inner();

    if let Some(file) = file {
        // a static file was found!
        tcp_stream.write_all("HTTP/1.1 200 OK\r\n".as_bytes())?;
        tcp_stream.write_all(format!("Content-Type: {}\r\n", mime_type(file)).as_bytes())?;
        tcp_stream.write_all("\r\n".as_bytes())?;
        send_file(file, &mut tcp_stream)?;
    } else if Path::new(&path).is_dir() {
        if !request.path.ends_with('/') {
            tcp_stream.write_all("HTTP/1.1 301 Moved Permanently\r\n".as_bytes())?;
            tcp_stream.write_all(format!("Location: {}/\r\n", request.path).as_bytes())?;
            tcp_stream.write_all("\r\n".as_bytes())?;
        } else {
            // try a directory listing
            tcp_stream.write_all("HTTP/1.1 200 OK\r\n".as_bytes())?;
            tcp_stream.write_all("Content-Type: text/html; charset=utf-8\r\n".as_bytes())?;
            tcp_stream.write_all("\r\n".as_bytes())?;
            tcp_stream.write_all(list_directory(&path)?.as_bytes())?;
        }
    } else {
        // nothing was found
        tcp_stream.write_all("HTTP/1.1 404 Not Found\r\n".as_bytes())?;
    }

    Ok(())
}

fn send_file(file: &str, tcp_stream: &mut TcpStream) -> Result<(), Box<dyn Error>> {
    let mut buffer = [0 as u8; 1024];
    let mut file = std::fs::File::open(file)?;
    while let bytes_read = file.read(&mut buffer)?
        && bytes_read != 0
    {
        tcp_stream.write_all(&buffer[..bytes_read])?;
    }
    Ok(())
}

fn parse_args() -> Config {
    let mut res = Config {
        port: DEFAULT_PORT,
        address: DEFAULT_ADDRESS.to_owned(),
        directory: DEFAULT_DIR.to_owned(),
    };

    let mut iter = std::env::args().skip(1);

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-b" => {
                let Some(arg_value) = iter.next() else {
                    panic!("'-b' needs a value")
                };
                res.address = arg_value;
            }
            "-p" => {
                let Some(arg_value) = iter.next() else {
                    panic!("'-p' needs a value")
                };
                res.port = arg_value.parse().expect("port number must be a u16");
            }
            "-d" => {
                let Some(arg_value) = iter.next() else {
                    panic!("'-d' needs a value")
                };
                res.directory = arg_value;
            }
            _ => panic!("bad option"),
        }
    }

    res
}
fn main() -> Result<(), Box<dyn Error>> {
    let config = parse_args();

    let listener = TcpListener::bind(format!("{}:{}", config.address, config.port))?;

    std::env::set_current_dir(&config.directory)
        .unwrap_or_else(|_| panic!("failed to move to '{}'", config.directory));

    println!("Listening on http://{}:{}", config.address, config.port);
    println!("serving out of {}", std::env::current_dir()?.display());

    loop {
        let (tcp_stream, _sock_addr) = listener.accept()?;

        process_request(tcp_stream)?;
    }
}

#[test]
fn test_normalize_path() {
    assert_eq!(
        normalize_path("../../../../../../..///etc///passwd".to_owned()),
        "etc/passwd"
    );
    assert_eq!(
        normalize_path("/./.././.././//././//./././tmp/././././".to_owned()),
        "tmp"
    );
    assert_eq!(normalize_path("/usr/bin/../lib//./".to_owned()), "usr/lib")
}
