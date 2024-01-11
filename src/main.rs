use std::fs::{read_to_string, write};
use std::net::SocketAddr;
use std::thread::spawn;
use coap_lite::{CoapRequest, RequestType};
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

#[tokio::main]
async fn main() -> Result<()> {
    let url_register = "coap://127.0.0.1:5683/actuator/register";

    let mut actuator_ip_address = String::new();
    let actuator_port = 5684i16;

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
            "ip_address": actuator_ip_address,
            "online": true,
            "state": false,
            "pulse": is_pulse,
            "port": actuator_port,
        }
    }.to_string().as_bytes().to_vec();

    let response_register = CoAPClient::post(url_register, register_params.clone()).unwrap();
    let new_actuator = String::from_utf8(response_register.message.payload).unwrap();

    if new_actuator == "KO" {
        println!("Error registering actuator");
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

                let response_register = CoAPClient::post(url_register, register_params.clone());

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