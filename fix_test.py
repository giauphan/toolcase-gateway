with open("src/tests.rs", "r") as f:
    text = f.read()

# Update the test to handle both wake AND token requests since we added wake in bootstrap
new_test = """    let server_thread = std::thread::spawn(move || {
        let (mut client, _) = mock_hatch.accept().unwrap();
        let request = crate::http::read_request(&mut client).unwrap();

        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/hatch/vm/wake");

        let response_body1 = r#"{"status":"wake_requested"}"#;
        use std::io::Write;
        write!(
            client,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body1.len(),
            response_body1
        ).unwrap();
        client.flush().unwrap();
        drop(client);
        
        // now accept the token request
        let (mut client, _) = mock_hatch.accept().unwrap();
        let request = crate::http::read_request(&mut client).unwrap();

        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/hatch/token");

        let response_body = r#"{"token":"test_access","notary_token":"test_notary"}"#;
        write!(
            client,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        ).unwrap();
        client.flush().unwrap();
    });"""

import re
text = re.sub(
    r'let server_thread = std::thread::spawn\(move \|\| \{.*?client\.flush\(\)\.unwrap\(\);\n    \}\);',
    new_test,
    text,
    flags=re.DOTALL
)

with open("src/tests.rs", "w") as f:
    f.write(text)
