// Import section: Essential dependencies for the server implementation
use crate::message::{AddRequest, AddResponse, ClientMessage, EchoMessage, ServerMessage};
use crate::message::client_message;
use crate::message::server_message;
use log::{error, info, warn};
use prost::Message;
use std::{
    io::{self, ErrorKind, Read, Write},
    net::{TcpListener, TcpStream, Shutdown},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, mpsc,Mutex
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use threadpool::ThreadPool;

// Configuration constants for server behavior
const BUFFER_SIZE: usize = 512;            // Fixed size for message buffers
const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);  // Timeout for client operations
const THREAD_POOL_SIZE: usize = 4;         // Number of worker threads in the pool
const SHUTDOWN_CHECK_INTERVAL: Duration = Duration::from_millis(100);  // Interval for checking shutdown signal
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);  // Maximum time to wait for shutdown

/// Client struct: Manages individual client connections
/// Maintains its own buffer to prevent race conditions
struct Client {
    stream: TcpStream,
    buffer: [u8; BUFFER_SIZE],
}

impl Client {
    /// Creates a new client instance with configured timeouts
    pub fn new(stream: TcpStream) -> io::Result<Self> {
        // Set timeouts to prevent hanging operations
        stream.set_read_timeout(Some(CLIENT_TIMEOUT))?;
        stream.set_write_timeout(Some(CLIENT_TIMEOUT))?;
        
        Ok(Client {
            stream,
            buffer: [0; BUFFER_SIZE],
        })
    }

    /// Handles client communication
    /// Returns Ok(true) to continue, Ok(false) to disconnect
    pub fn handle(&mut self) -> io::Result<bool> {
        // Read data from client with sophisticated disconnection detection
        let bytes_read = match self.stream.read(&mut self.buffer) {
            Ok(0) => {
                // Implement graceful disconnection with waiting period
                info!("No data received. Waiting before closing the connection."); 
                thread::sleep(Duration::from_secs(2)); // Wait period to confirm disconnection
                if self.stream.read(&mut self.buffer)? == 0 { 
                    info!("Client disconnected after waiting."); 
                    return Ok(false) // Confirm disconnection
                } else { 
                    info!("Data received after waiting. Continuing handling."); 
                    return Ok(true) // Resume normal operation
                }
            }
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(true),
            Err(e) if e.kind() == ErrorKind::TimedOut => {
                warn!("Client operation timed out");
                return Ok(true);
            }
            Err(e) => {
                error!("Error reading from client: {}", e);
                return Err(e);
            }
        };

        // Process received message
        if let Ok(client_message) = ClientMessage::decode(&self.buffer[..bytes_read]) {
            // Handle different message types
            let response = match client_message.message {
                Some(client_message::Message::EchoMessage(echo)) => {
                    info!("Received echo request: {}", echo.content);
                    ServerMessage {
                        message: Some(server_message::Message::EchoMessage(echo)),
                    }
                }
                Some(client_message::Message::AddRequest(add)) => {
                    info!("Received add request: {} + {}", add.a, add.b);
                    let result = add.a + add.b;
                    ServerMessage {
                        message: Some(server_message::Message::AddResponse(AddResponse {
                            result,
                        })),
                    }
                }
                None => {
                    error!("Received message with no content");
                    return Ok(true);
                }
            };

            // Ensure response fits in buffer
            let payload = response.encode_to_vec();
            if payload.len() > BUFFER_SIZE {
                error!("Response too large for buffer");
                return Ok(true);
            }

            // Send response to client
            match self.stream.write_all(&payload) {
                Ok(_) => self.stream.flush()?,
                Err(e) => {
                    error!("Error writing to client: {}", e);
                    return Err(e);
                }
            }
        } else {
            error!("Failed to decode client message");
        }

        Ok(true)
    }

    /// Gracefully shuts down the client connection
    /// suppress_errors: Controls whether to log shutdown errors
    pub fn shutdown(&mut self, suppress_errors: bool) -> io::Result<()> {
        if let Err(e) = self.stream.shutdown(std::net::Shutdown::Both) {
            if !suppress_errors {
                error!("Error shutting down client connection: {}", e);
            }
        }
        Ok(())
    }
}

/// Server struct: Main server implementation
/// Manages the TCP listener, thread pool, and shutdown coordination
pub struct Server {
    listener: TcpListener,
    is_running: Arc<AtomicBool>,          // Thread-safe running flag
    thread_pool: ThreadPool,               // Worker thread pool
    main_thread: Option<JoinHandle<io::Result<()>>>,  // Main server thread handle
    shutdown_sender: mpsc::Sender<()>,     // Shutdown signal sender
    shutdown_receiver: Arc<Mutex<mpsc::Receiver<()>>>, // Thread-safe shutdown receiver
}

impl Server {
    /// Creates a new server instance bound to the specified address
    pub fn new(addr: &str) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let is_running = Arc::new(AtomicBool::new(false));
        let thread_pool = ThreadPool::new(THREAD_POOL_SIZE);
        let (shutdown_sender, shutdown_receiver) = mpsc::channel();

        Ok(Server {
            listener,
            is_running,
            thread_pool,
            main_thread: None,
            shutdown_sender,
            shutdown_receiver: Arc::new(Mutex::new(shutdown_receiver)),
        })
    }

    /// Runs the server in a new thread
    /// Returns immediately while server runs in background
    pub fn run(&mut self) -> io::Result<()> {
        // Prevent multiple run calls
        if self.is_running.load(Ordering::SeqCst) {
            warn!("Server is already running");
            return Ok(());
        }

        // Initialize server state
        self.is_running.store(true, Ordering::SeqCst);
        let is_running = Arc::clone(&self.is_running);
        let listener = self.listener.try_clone()?;
        let shutdown_receiver = Arc::clone(&self.shutdown_receiver);

        // Spawn main server thread
        let thread = thread::spawn(move || -> io::Result<()> {
            info!("Server starting on {} with {} worker threads", 
                  listener.local_addr()?, 
                  THREAD_POOL_SIZE);

            listener.set_nonblocking(true)?;
            let pool = ThreadPool::new(THREAD_POOL_SIZE);

            // Main server loop
            while is_running.load(Ordering::SeqCst) {
                // Check for shutdown signal
                if let Ok(_) = shutdown_receiver.lock().unwrap().try_recv() {
                    info!("Shutdown signal received in main thread");
                    break;
                }

                // Handle new connections
                match listener.accept() {
                    Ok((stream, addr)) => {
                        info!("New client connected: {}", addr);
                        let is_running = Arc::clone(&is_running);
                        
                        // Spawn client handler in thread pool
                        pool.execute(move || {
                            if let Ok(mut client) = Client::new(stream) {
                                while is_running.load(Ordering::SeqCst) {
                                    match client.handle() {
                                        Ok(true) => continue,
                                        Ok(false) | Err(_) => break,
                                    }
                                }
                                // Cleanup client connection
                                if let Err(e) = client.shutdown(false) {
                                    warn!("Failed shutting down client connection: {}", e);
                                }
                            }
                            info!("Client handler thread finished for {}", addr);
                        });
                    }
                    Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(SHUTDOWN_CHECK_INTERVAL);
                    }
                    Err(e) => {
                        error!("Error accepting connection: {}", e);
                    }
                }
            }

            // Cleanup phase
            info!("Main server loop ending, waiting for thread pool to complete...");
            pool.join();
            info!("All client threads finished");
            Ok(())
        });

        self.main_thread = Some(thread);
        Ok(())
    }

    /// Initiates graceful server shutdown
    pub fn stop(&mut self) -> io::Result<()> {
        if !self.is_running.load(Ordering::SeqCst) {
            warn!("Server is not running");
            return Ok(());
        }
    
        info!("Initiating server shutdown...");
        
        // Signal shutdown
        self.is_running.store(false, Ordering::SeqCst);
    
        // Wait for main thread completion
        if let Some(thread) = self.main_thread.take() {
            match thread.join() {
                Ok(_) => info!("Server shutdown completed successfully"),
                Err(e) => error!("Error during server shutdown: {:?}", e),
            }
        } else {
            warn!("No main thread to join, shutdown may be incomplete");
        }
    
        Ok(())
    }
}

/// Implements automatic cleanup when Server is dropped
impl Drop for Server {
    fn drop(&mut self) {
        if self.is_running.load(Ordering::SeqCst) {
            if let Err(e) = self.stop() {
                error!("Error during server cleanup: {}", e);
            }
        }
    }
}