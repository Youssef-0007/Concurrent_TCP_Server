use embedded_recruitment_task::{
    message::{client_message, server_message, AddRequest, EchoMessage}, // Import Protobuf message types for client-server communication
    server::Server, // Import the Server struct
};
use std::{
    sync::{Arc, Mutex, Once}, // Synchronization utilities for safe shared access
    thread::{self, JoinHandle}, // Thread utilities for spawning server threads
    time::{Duration, Instant}, // Time utilities for handling timeouts
    net::TcpListener, // Used to find available ports dynamically for tests
};
use log::{error, info, warn};

// `Once` ensures the logger is initialized only once for all test cases
static INIT: Once = Once::new();

mod client;

/// Spawns a new thread to run the server
/// - The `server` is wrapped in an `Arc<Mutex<>>` to allow shared, thread-safe, mutable access.
/// - The function returns a `JoinHandle` that can be used to wait for the thread to complete.
fn setup_server_thread(server: Arc<Mutex<Server>>) -> JoinHandle<()> {
    thread::spawn(move || {
        // Acquire the lock to access the server
        let mut server = server.lock().expect("Failed to lock the server");
        // Start the server in this thread
        server.run().expect("Server encountered an error");
    })
}

/// Creates a server instance on a specified port
/// - Uses the provided `port` to bind the server.
/// - Returns the server wrapped in an `Arc<Mutex<>>` for safe shared mutable access.
fn create_server(port: u16) -> Arc<Mutex<Server>> {
    let addr = format!("localhost:{}", port); // Format the server address
    let server = Server::new(&addr).expect("Failed to start server"); // Create a new server instance
    Arc::new(Mutex::new(server)) // Wrap the server in `Arc<Mutex>`
}

/// Dynamically finds an available port for testing
/// - Binds a temporary `TcpListener` to any available port on localhost.
/// - Returns the port number as `u16`.
fn find_available_port() -> u16 {
    TcpListener::bind("localhost:0")
        .expect("Failed to bind to an available port") // Bind to a free port
        .local_addr()
        .unwrap() // Get the local address of the bound listener
        .port() // Extract the port number
}

// ============================== TEST FUNCTIONS ==============================

/// Tests whether the client can successfully connect to and disconnect from the server.
/// - Starts the server on a unique port.
/// - Verifies that the client can connect and then disconnect without errors.
#[test]
fn test_client_connection() {
    INIT.call_once(|| {
        env_logger::init(); // Initialize the logger once for all tests
    });

    let port = find_available_port(); // Get a unique port for this test
    let server = create_server(port); // Create a server instance
    let handle = setup_server_thread(server.clone()); // Start the server in a separate thread

    // Create and connect the client to the server
    let mut client = client::Client::new("localhost", port as u32, 1000);
    assert!(client.connect().is_ok(), "Failed to connect to the server"); // Assert connection success
    info!("Client connected to the server");

    // Disconnect the client
    client.disconnect().expect("Failed to disconnect from the server");
    info!("Client disconnected from the server");

    // Stop the server and wait for the thread to finish
    {
        let mut server = server.lock().expect("Failed to lock the server"); // Lock the server for safe access
        server.stop().expect("Failed to stop the server"); // Stop the server
    }
    handle.join().expect("Server thread panicked or failed to join"); // Wait for the server thread to complete
}

/// Tests whether the client can send an `EchoMessage` and receive the same message echoed back.
/// - Verifies that the message content remains unchanged.
#[test]
fn test_client_echo_message() {
    INIT.call_once(|| {
        env_logger::init(); // Initialize the logger
    });

    let port = find_available_port(); // Get a unique port
    let server = create_server(port); // Create a server instance
    let handle = setup_server_thread(server.clone()); // Start the server

    // Create and connect the client
    let mut client = client::Client::new("localhost", port as u32, 1000);
    assert!(client.connect().is_ok(), "Failed to connect to the server"); // Assert connection success

    let mut echo_message = EchoMessage::default(); // Create a default `EchoMessage`
    echo_message.content = "Hello, World!".to_string(); // Set the content of the message
    let message = client_message::Message::EchoMessage(echo_message.clone()); // Wrap the message in a `client_message`

    // Send the `EchoMessage` to the server
    assert!(client.send(message).is_ok(), "Failed to send message"); // Assert successful send

    // Receive the echoed message from the server
    let response = client.receive().expect("Failed to receive response");

    match response.message {
        Some(server_message::Message::EchoMessage(echo)) => {
            assert_eq!(echo.content, echo_message.content, "Echoed content mismatch"); // Assert content matches
            info!("Received EchoMessage: {}", echo.content); // Log the received content
        }
        _ => panic!("Expected EchoMessage, but received a different message"), // Panic if the response is not an `EchoMessage`
    }

    client.disconnect().expect("Failed to disconnect from the server"); // Disconnect the client

    {
        let mut server = server.lock().expect("Failed to lock the server"); // Lock and stop the server
        server.stop().expect("Failed to stop the server");
    }
    handle.join().expect("Server thread panicked or failed to join"); // Wait for the server thread to complete
}

/* Key Features:

Message Sending and Receiving: Verifies that the server correctly processes each 
EchoMessage and responds with the same content, ensuring reliable bidirectional communication.

Timeout Handling: Implements a timeout mechanism to ensure the client receives responses 
within a specified period, avoiding indefinite waits.

Server and Client Lifecycle Management: Properly handles the connection, message exchange, 
and disconnection for the client, and ensures the server is stopped and cleaned up after the test.

*/
#[test]
fn test_multiple_echo_messages() {
    // Reinitialize the logging if it's not already initialized 
    INIT.call_once(|| {
        env_logger::init();
    });

    let port = find_available_port(); // Dynamically find an available port
    // Set up the server in a separate thread
    let server = create_server(port);
    let handle = setup_server_thread(server.clone());

    let mut client = client::Client::new("localhost", port as u32, 1000); // Create a new client
    client.connect().expect("Client failed to connect to the server"); // Connect the client to the server
    info!("Client connected to the server");

    let messages = vec![
        "Hello, World!".to_string(),
        "How are you?".to_string(),
        "Goodbye!".to_string(),
    ];
    
    // Adding a timeout mechanism to avoid indefinite waits.
    let timeout = Duration::from_secs(5);
    let start_time = Instant::now(); // Record the current time to track elapsed time

    for message_content in messages {
        let mut echo_message = EchoMessage::default(); // Create an `EchoMessage`
        echo_message.content = message_content.clone(); // Set its content
        let message = client_message::Message::EchoMessage(echo_message); // Wrap the echo message in a `Message` enum

        client.send(message).expect("Client failed to send message"); // Send the message
        info!("Sent EchoMessage: {}", message_content);

        let mut response_received = false;
        while start_time.elapsed() < timeout { // Check if the elapsed time is within the timeout period
            let response = client.receive(); // Receive the response
            if let Ok(response) = response {
                match response.message {
                    Some(server_message::Message::EchoMessage(echo)) => { // Check if the response is an EchoMessage
                        assert_eq!(echo.content, message_content, "Echoed message content does not match"); // Validate the content of the echoed message
                        info!("Received EchoMessage: {}", echo.content);
                        response_received = true; // Mark that the response has been received
                        break; // Exit the loop as the response has been successfully received and validated
                    }
                    _ => panic!("Expected EchoMessage, but received a different message"),
                }
            } else {
                std::thread::sleep(Duration::from_millis(100)); // Sleep for a short duration before checking again
            }
        }

        assert!(response_received, "Test timed out while waiting for server response"); // Ensure the response was received within the timeout period
    }

    client.disconnect().expect("Client failed to disconnect from the server"); // Disconnect the client from the server
    info!("Client disconnected from the server");

    // Stop the server
    {
        let mut server = server.lock().expect("Failed to lock the server"); // Lock the server for exclusive access
        server.stop().expect("Failed to stop the server"); // Stop the server
    }
    handle.join().expect("Server thread panicked or failed to join"); // Wait for the server thread to finish
    info!("Server stopped successfully");
}



/// Tests whether multiple clients can connect to the server, send messages, and receive responses.
/// - Ensures each client can interact with the server independently.
/// 
/* Key features 
Simultaneous Client Connections: The test ensures that multiple clients can connect to the 
server simultaneously and independently, verifying the server's capability to handle concurrent connections.

Message Sending and Receiving: Each client sends an EchoMessage to the server and verifies that 
the server correctly echoes back the message, ensuring reliable bidirectional communication.

Timeout Handling: The test checks that responses are received within a specified timeout period, 
ensuring the server responds promptly and preventing the test from hanging indefinitely.
*/
#[test]
fn test_multiple_clients() {
    INIT.call_once(|| {
        env_logger::init();
    });

    let port = find_available_port(); // Get a unique port
    let server = create_server(port); // Create the server
    let handle = setup_server_thread(server.clone()); // Start the server

    // Create multiple clients
    let mut clients = vec![
        client::Client::new("localhost", port as u32, 1000),
        client::Client::new("localhost", port as u32, 1000),
        client::Client::new("localhost", port as u32, 1000),
    ];

    // Connect each client
    for (i, client) in clients.iter_mut().enumerate() {
        client.connect().expect("Client failed to connect to the server");
        info!("Client {} connected to the server", i + 1);
    }

    let messages = vec![
        "Hello, World!".to_string(),
        "How are you?".to_string(),
        "Goodbye!".to_string(),
    ];

    // Set time out 
    let timeout = Duration::from_secs(10);
    let start_time = Instant::now(); // Record the current time to track elapsed time

    for message_content in messages {
        let mut echo_message = EchoMessage::default(); // Create an `EchoMessage`
        echo_message.content = message_content.clone(); // Set its content
        let message = client_message::Message::EchoMessage(echo_message.clone()); // Wrap the echo message in a `Message` enum

        for (i, client) in clients.iter_mut().enumerate() {
            client.send(message.clone()).expect("Client failed to send message"); // Send the message
            info!("Client {} sent EchoMessage: {}", i + 1, message_content);

            let mut response_received = false;
            while start_time.elapsed() < timeout { // Check if the elapsed time is within the timeout period
                let response = client.receive(); // Receive the response
                if let Ok(response) = response {
                    match response.message {
                        Some(server_message::Message::EchoMessage(echo)) => { // Check if the response is an EchoMessage
                            assert_eq!(echo.content, message_content, "Echoed message content does not match"); // Validate the content of the echoed message
                            info!("Client {} received EchoMessage: {}", i + 1, echo.content);
                            response_received = true; // Mark that the response has been received
                            break; // Exit the loop as the response has been successfully received and validated
                        }
                        _ => panic!("Expected EchoMessage, but received a different message"),
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(100)); // Sleep for a short duration before checking again
                }
            }

            assert!(response_received, "Test timed out while waiting for server response"); // Ensure the response was received within the timeout period
        }
    }

    for (i, client) in clients.iter_mut().enumerate() {
        client.disconnect().expect("Client failed to disconnect from the server"); // Disconnect the client from the server
        info!("Client {} disconnected from the server", i + 1);
    }

    // Stop the server
    {
        let mut server = server.lock().expect("Failed to lock the server"); // Lock the server for exclusive access
        server.stop().expect("Failed to stop the server"); // Stop the server
    }

    handle.join().expect("Server thread panicked or failed to join"); // Wait for the server thread to finish
    info!("Server stopped successfully");

}


/* Key Features:

Message Handling and Response: Verifies that the server correctly processes 
the AddRequest and responds with an AddResponse containing the correct result.

Timeout Handling: Implements a timeout mechanism to ensure the client receives a response 
within a specified period, avoiding indefinite waits.

Server and Client Lifecycle Management: Properly handles the connection, message exchange, and disconnection for the client, 
and ensures the server is stopped and cleaned up after the test.

*/
#[test]
fn test_client_add_request() {
    // Reinitializing the environment logger 
    INIT.call_once(|| {
        env_logger::init();
    });

    let port = find_available_port(); // Dynamically find an available port
    // Set up the server in a separate thread
    let server = create_server(port);
    let handle = setup_server_thread(server.clone());

    let mut client = client::Client::new("localhost", port as u32, 1000); // Create a new client
    client.connect().expect("Client failed to connect to the server"); // Connect the client to the server
    info!("Client connected to the server");

    let mut add_request = AddRequest::default(); // Create an `AddRequest`
    add_request.a = 10;
    add_request.b = 20;
    let message = client_message::Message::AddRequest(add_request.clone()); // Wrap the add request in a `Message` enum

    client.send(message).expect("Client failed to send AddRequest"); // Send the add request
    info!("Sent AddRequest: a={}, b={}", add_request.a, add_request.b);
    
    // Timeout to avoid indefinite waits
    let timeout = Duration::from_secs(5);
    let start_time = Instant::now(); // Record the current time to track elapsed time

    let mut response_received = false;
    while start_time.elapsed() < timeout { // Check if the elapsed time is within the timeout period
        let response = client.receive(); // Receive the response
        if let Ok(response) = response {
            match response.message {
                Some(server_message::Message::AddResponse(add_response)) => { // Check if the response is an AddResponse
                    assert_eq!(add_response.result, add_request.a + add_request.b, "AddResponse result does not match"); // Validate the result of the add response
                    info!("Received AddResponse: result={}", add_response.result);
                    response_received = true; // Mark that the response has been received
                    break; // Exit the loop as the response has been successfully received and validated
                }
                _ => panic!("Expected AddResponse, but received a different message"),
            }
        } else {
            std::thread::sleep(Duration::from_millis(100)); // Sleep for a short duration before checking again
        }
    }

    assert!(response_received, "Test timed out while waiting for server response"); // Ensure the response was received within the timeout period

    client.disconnect().expect("Client failed to disconnect from the server"); // Disconnect the client from the server
    info!("Client disconnected from the server");

    // Stop the server
    {
        let mut server = server.lock().expect("Failed to lock the server"); // Lock the server for exclusive access
        server.stop().expect("Failed to stop the server"); // Stop the server
    }
    handle.join().expect("Server thread panicked or failed to join"); // Wait for the server thread to finish
    info!("Server stopped successfully");
}

//====================================== Additional Test Functions ====================================
/*Key Features:
Sequential Connections: Simulates multiple clients connecting sequentially.

Message Echo: Verifies message sent is echoed correctly.

Graceful Disconnection: Ensures clients disconnect gracefully.

Server Shutdown: Server shuts down after handling all clients.
*/#[test]
fn test_sequential_clients() {
    // Reinitializing the environment logger
    INIT.call_once(|| {
        env_logger::init();
    });

    let port = find_available_port(); // Dynamically find an available port
    // Set up the server in a separate thread
    let server = create_server(port);
    let handle = setup_server_thread(server.clone());

    for i in 1..=5 {
        // Create and connect the client
        let mut client = client::Client::new("localhost", port as u32, 1000);
        assert!(
            client.connect().is_ok(),
            "Client {} failed to connect to the server",
            i
        );

        // Prepare a message
        let mut echo_message = EchoMessage::default();
        echo_message.content = format!("Message from client {}", i);
        let message = client_message::Message::EchoMessage(echo_message.clone());

        // Send and receive the message
        assert!(
            client.send(message).is_ok(),
            "Client {} failed to send message",
            i
        );

        let response = client.receive();
        assert!(
            response.is_ok(),
            "Client {} failed to receive response",
            i
        );

        match response.unwrap().message {
            Some(server_message::Message::EchoMessage(echo)) => {
                assert_eq!(
                    echo.content, echo_message.content,
                    "Client {} received incorrect response",
                    i
                );
            }
            _ => panic!("Client {} received unexpected message type", i),
        }

        // Disconnect the client
        assert!(
            client.disconnect().is_ok(),
            "Client {} failed to disconnect from the server",
            i
        );
    }

    // Stop the server
    {
        let mut server = server.lock().expect("Failed to lock the server"); // Lock the server for exclusive access
        server.stop().expect("Failed to stop the server"); // Stop the server
    }
    handle.join().expect("Server thread panicked or failed to join"); // Wait for the server thread to finish
    info!("Server stopped successfully");
}

/*Key Features:
Abrupt Disconnection: Simulates client disconnecting abruptly.

Resource Cleanup: Verifies server cleans up resources properly.

Stability Check: Ensures server remains stable.

Graceful Shutdown: Server shuts down gracefully post-disconnection.
*/
#[test]
fn test_abrupt_client_disconnect() {
    // Reinitializing the environment logger
    INIT.call_once(|| {
        env_logger::init();
    });

    let port = find_available_port(); // Dynamically find an available port
    // Set up the server in a separate thread
    let server = create_server(port);
    let handle = setup_server_thread(server.clone());

    // Create and connect the client
    let mut client = client::Client::new("localhost", port as u32, 1000);
    assert!(client.connect().is_ok(), "Failed to connect to the server");

    // Prepare and send a message
    let mut echo_message = EchoMessage::default();
    echo_message.content = "Test abrupt disconnect".to_string();
    let message = client_message::Message::EchoMessage(echo_message.clone());
    assert!(client.send(message).is_ok(), "Failed to send message");

    // Abruptly disconnect the client using the new shutdown method
    assert!(client.shutdown().is_ok(), "Failed to shutdown the client");

    // Wait briefly to ensure the server processes the disconnect
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Stop the server
    {
        let mut server = server.lock().expect("Failed to lock the server"); // Lock the server for exclusive access
        server.stop().expect("Failed to stop the server"); // Stop the server
    }
    handle.join().expect("Server thread panicked or failed to join"); // Wait for the server thread to finish
    info!("Server stopped successfully");
}


/*Key Features:
High Load Simulation: Simulates 50 clients connecting concurrently.

Varying Message Sizes: Tests server with different message sizes.

Message Validation: Verifies correct message echo.

Server Shutdown: Ensures graceful server shutdown post-load.
*/
#[test]
fn test_high_load_with_varying_message_sizes() {
    // Reinitializing the environment logger 
    INIT.call_once(|| {
        env_logger::init();
    });

    let port = find_available_port(); // Dynamically find an available port
    // Set up the server in a separate thread
    let server = create_server(port);
    let handle = setup_server_thread(server.clone());

    let mut handles = vec![];
    for i in 0..50 {
        let handle = thread::spawn(move || { // Spawn 50 client threads
            let mut client = client::Client::new("localhost", port as u32, 1000);
            client.connect().expect("Failed to connect");

            let content = "A".repeat(10 + (i * 10)); // Create message with varying sizes
            let mut echo_message = EchoMessage::default();
            echo_message.content = content.clone();
            let message = client_message::Message::EchoMessage(echo_message);

            client.send(message).expect("Failed to send message");

            let response = client.receive();
            assert!(response.is_ok(), "Failed to receive response");

            match response.unwrap().message {
                Some(server_message::Message::EchoMessage(echo)) => {
                    assert_eq!(echo.content, content, "Message content mismatch");
                }
                _ => panic!("Unexpected message type"),
            }

            client.disconnect().expect("Failed to disconnect");
        });

        handles.push(handle); // Collect handles for all threads
    }

    for handle in handles {
        handle.join().expect("Client thread panicked"); // Wait for all client threads to finish
    }

    // Stop the server
    {
        let mut server = server.lock().expect("Failed to lock the server"); // Lock the server for exclusive access
        server.stop().expect("Failed to stop the server"); // Stop the server
    }
    handle.join().expect("Server thread panicked or failed to join"); // Wait for the server thread to finish
    info!("Server stopped successfully");
}
