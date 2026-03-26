use std::fs::{read_to_string, write};
use std::net::SocketAddr;
use std::thread::spawn;
use std::time::Duration;
use coap_lite::{CoapRequest, MessageType, RequestType};
use serde_json::json;
use local_ip_address::local_ip;
use fake_actuator::{CoAPClient, Server};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct RegisterResponse {
    id: i32,
    state: bool,
}

fn discover_backend(default_port: u16) -> (String, u16) {
    println!("Discovering backend via CoAP multicast...");

    let client = match CoAPClient::new(("224.0.1.187", default_port)) {
        Ok(c) => c,
        Err(e) => {
            println!("Failed to create discovery client: {:?}, using fallback", e);
            return ("127.0.0.1".to_string(), default_port);
        }
    };

    client.set_broadcast(true).ok();
    let _ = client.set_receive_timeout(Some(Duration::from_secs(3)));

    let mut request = CoapRequest::<SocketAddr>::new();
    request.set_method(RequestType::Get);
    request.set_path("/.well-known/core");
    request
        .message
        .header
        .set_type(MessageType::NonConfirmable);

    if let Err(e) = client.send_all_coap(&request, 0) {
        println!("Failed to send multicast: {:?}, using fallback", e);
        return ("127.0.0.1".to_string(), default_port);
    }

    match client.receive() {
        Ok(response) => {
            let payload = String::from_utf8_lossy(&response.message.payload);
            println!("Discovery response: {}", payload);
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&payload) {
                let ip = parsed["ip"].as_str().unwrap_or("127.0.0.1").to_string();
                let port = parsed["coap_port"].as_u64().unwrap_or(default_port as u64) as u16;
                println!("Discovered backend at {}:{}", ip, port);
                return (ip, port);
            }
            println!("Failed to parse discovery response, using fallback");
            ("127.0.0.1".to_string(), default_port)
        }
        Err(e) => {
            println!("No discovery response: {:?}, using fallback", e);
            ("127.0.0.1".to_string(), default_port)
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let (backend_ip, backend_port) = discover_backend(5683);
    let url_register = format!("coap://{}:{}/actuator/register", backend_ip, backend_port);

    let mut actuator_ip_address = String::new();
    let actuator_port: i16 = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "8684".to_string())
        .parse()
        .expect("Invalid port number");
    let device_secret = std::env::args().nth(2).unwrap_or_else(|| {
        eprintln!("Warning: no device_secret provided (arg 2)");
        String::new()
    });

    match local_ip() {
        Ok(ip) => {
            actuator_ip_address.push_str(ip.to_string().as_str());
        }
        Err(_) => panic!("Unable to get local IP address")
    }

    println!("Local IP address: {}", actuator_ip_address);

    let is_pulse = match read_to_string(".is_pulse") {
        Ok(_) => true,
        Err(_) => false
    };

    let register_params = json! {
        {
            "device_secret": device_secret,
            "ip_address": actuator_ip_address,
            "online": true,
            "state": false,
            "pulse": is_pulse,
            "port": actuator_port,
        }
    }.to_string().as_bytes().to_vec();

    let response_register = CoAPClient::post(&url_register, register_params.clone()).unwrap();
    let new_actuator = String::from_utf8(response_register.message.payload).unwrap();

    if new_actuator == "KO" || new_actuator == "Unauthorized" {
        println!("Error registering actuator: {}", new_actuator);
        return Ok(());
    }

    let register_response: RegisterResponse = serde_json::from_str(new_actuator.as_str()).expect("Unable to parse JSON");

    write(".status", if register_response.state { "ON" } else { "OFF" }).expect("Unable to write file");

    spawn(move || {
        loop {
            let time = read_to_string(".time").unwrap_or_else(|_| "0".to_string());
            let time_as_int = time.parse::<u64>().unwrap_or_else(|_| 0);

            if time_as_int > 60 {
                println!("Actuator offline, trying to re-register");

                let response_register = CoAPClient::post(&url_register, register_params.clone());

                match response_register {
                    Ok(_) => {
                        println!("Actuator re-registered");
                        write(".time", 0u64.to_string()).unwrap_or_else(|_| {});
                    }
                    Err(_) => {
                        std::thread::sleep(std::time::Duration::from_secs(59));
                    }
                }
            } else {
                write(".time", (time_as_int + 1).to_string()).unwrap_or_else(|_| {});
            }

            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    });

    run_server(actuator_ip_address, actuator_port).await;

    Ok(())
}

async fn run_server(actuator_ip_address: String, actuator_port: i16) {
    let address = actuator_ip_address + ":" + actuator_port.to_string().as_str();

    println!("Running server on {}", address);

    let mut server = Server::new(address).unwrap();

    server.run(
        |request| async {
            let request_ref = &request;

            let payload = callback(request_ref).await;

            println!("State: {}", payload);

            write(".time", 0u64.to_string()).unwrap_or_else(|_| {});

            match request.response {
                Some(mut message) => {
                    message.message.payload = payload.as_bytes().to_vec();

                    Some(message)
                }
                _ => None,
            }
        },
    )
        .await
        .expect("Failed to create server");
}

async fn callback(request: &CoapRequest<SocketAddr>) -> String {

    if request.get_method() != &RequestType::Post && request.get_method() != &RequestType::Get {
        return "KO".to_string();
    }

    if request.get_method() == &RequestType::Get {
        return read_to_string(".status").unwrap_or_else(|_| "KO".to_string());
    }

    let payload = String::from_utf8(request.message.payload.clone()).unwrap();

    if payload == "ON" {
        match write(".status", "ON") {
            Ok(_) => {}
            Err(_) => {}
        };
        "ON".to_string()
    } else if payload == "ON-PULSE" {
        spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(750));

            match write(".status", "OFF") {
                Ok(_) => {}
                Err(_) => {}
            };
        });
        match write(".status", "ON-PULSE") {
            Ok(_) => {}
            Err(_) => {}
        };
        "ON-PULSE".to_string()
    } else if payload == "OFF" {
        match write(".status", "OFF") {
            Ok(_) => {}
            Err(_) => {}
        };
        "OFF".to_string()
    } else {
        "KO".to_string()
    }
}