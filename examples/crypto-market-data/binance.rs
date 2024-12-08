use disruptor_rs::EventProducer;
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tungstenite::stream::MaybeTlsStream;

const ENDPOINT: &str = "wss://fstream.binance.com/ws";

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(untagged)]
pub enum BinanceMessage {
    #[default]
    None,

    #[serde(rename = "aggTrade")]
    AggTrade(AggTradeData),

    #[serde(rename = "depthUpdate")]
    Depth(DepthData),

    SubscriptionResponse {
        result: Option<String>,
        id: u64,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AggTradeData {
    #[serde(rename = "e")]
    pub event_type: String, // Event type
    #[serde(rename = "E")]
    pub event_time: u64, // Event time
    #[serde(rename = "s")]
    pub symbol: String, // Symbol
    #[serde(rename = "a")]
    pub agg_trade_id: u64, // Aggregate trade ID
    #[serde(rename = "p")]
    pub price: String, // Price
    #[serde(rename = "q")]
    pub quantity: String, // Quantity
    #[serde(rename = "f")]
    pub first_trade_id: u64, // First trade ID
    #[serde(rename = "l")]
    pub last_trade_id: u64, // Last trade ID
    #[serde(rename = "T")]
    pub trade_time: u64, // Trade time
    #[serde(rename = "m")]
    pub is_buyer_maker: bool, // Is the buyer the market maker?
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct DepthData {
    #[serde(rename = "e")]
    pub event_type: String, // Event type
    #[serde(rename = "E")]
    pub event_time: u64, // Event time
    #[serde(rename = "s")]
    pub symbol: String, // Symbol
    #[serde(rename = "U")]
    pub first_update_id: u64, // First update ID
    #[serde(rename = "u")]
    pub final_update_id: u64, // Final update ID
    #[serde(rename = "b")]
    pub bids: Vec<Vec<String>>, // Bids
    #[serde(rename = "a")]
    pub asks: Vec<Vec<String>>, // Asks
}

fn parse_message(message: &str) -> Result<BinanceMessage, serde_json::Error> {
    serde_json::from_str(message)
}

pub trait WebsocketSubscriptionHandler {
    fn start(&self);
    fn stop(&mut self);
    fn run(&mut self);
    fn connect(&mut self);
    fn disconnect(&mut self);
    fn on_stop(&mut self);
    fn on_message(&mut self, message: tungstenite::Message);
    fn on_open(&mut self);
    fn on_close(&self);
    fn on_error(&self, error: &str);
    fn on_ping(&mut self, data: &[u8]);
}

pub struct BinanceWebsocketHandler<P: EventProducer<'static, Item = BinanceMessage>> {
    _symbols: Vec<String>,
    streams: Vec<String>,
    running: Arc<AtomicBool>,
    socket: Option<tungstenite::WebSocket<MaybeTlsStream<std::net::TcpStream>>>,
    producer: Arc<P>,
}

impl<P: EventProducer<'static, Item = BinanceMessage>> BinanceWebsocketHandler<P> {
    pub fn new(symbols: &[String], running: Arc<AtomicBool>, producer: Arc<P>) -> Self {
        Self {
            _symbols: symbols.to_vec(),
            streams: symbols
                .iter()
                .flat_map(|s| vec![format!("{s}@aggTrade"), format!("{s}@depth")])
                .collect(),
            running,
            socket: None,
            producer,
        }
    }

    fn subscribe_message(&self) -> String {
        format!(
            "{{\"method\": \"SUBSCRIBE\", \"params\": {}, \"id\": {}}}",
            serde_json::to_string(&self.streams).unwrap(),
            rand::random::<u8>()
        )
    }
}

impl<P: EventProducer<'static, Item = BinanceMessage>> WebsocketSubscriptionHandler
    for BinanceWebsocketHandler<P>
{
    fn start(&self) {
        self.running
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn run(&mut self) {
        while self.running.load(std::sync::atomic::Ordering::SeqCst) {
            if let Some(ref mut socket) = self.socket {
                let message = socket.read_message();
                match message {
                    Ok(message) => self.on_message(message),
                    Err(e) => self.on_error(&format!("Error reading message: {e}")),
                }
            } else {
                println!("No socket, connecting...");
                self.connect();
            }
        }
    }

    fn stop(&mut self) {
        println!("Stopping...");
        self.on_stop();
    }

    fn on_message(&mut self, message: tungstenite::Message) {
        match message {
            tungstenite::Message::Text(text) => match parse_message(&text) {
                Ok(message) => match message {
                    BinanceMessage::AggTrade(data) => {
                        self.producer.moving_write(vec![data], |slot, _, data| {
                            *slot = BinanceMessage::AggTrade(data);
                        });
                    }
                    BinanceMessage::Depth(data) => {
                        self.producer.moving_write(vec![data], |slot, _, data| {
                            *slot = BinanceMessage::Depth(data);
                        });
                    }
                    BinanceMessage::SubscriptionResponse { result, id } => {
                        println!("Subscription response - ID: {}, Result: {:?}", id, result);
                    }
                    _ => println!("Received unknown Binance message: {:?}", message),
                },
                Err(error) => self.on_error(&error.to_string()),
            },
            tungstenite::Message::Ping(ping) => self.on_ping(&ping),
            tungstenite::Message::Close(_) => {
                println!("Connection closed by server. Reconnecting...");
                self.disconnect();
                self.connect();
            }
            _ => println!("Received unknown message: {:?}", message),
        }
    }

    fn connect(&mut self) {
        let client_result = tungstenite::client::connect(ENDPOINT);
        println!("Connection established: {:?}", client_result.is_ok());
        match client_result {
            Ok((socket, response)) => {
                println!("Connection opened: {:?}", response);
                self.socket = Some(socket);
                self.on_open();
            }
            Err(e) => self.on_error(&format!("Error connecting to websocket: {e}")),
        }
    }

    fn disconnect(&mut self) {
        if let Some(mut socket) = self.socket.take() {
            match socket.close(None) {
                Ok(_) => self.on_close(),
                Err(e) => self.on_error(&e.to_string()),
            }
        }
    }

    fn on_open(&mut self) {
        let subscribe_message = self.subscribe_message();
        println!("Connecting to streams: {}", subscribe_message);

        if let Some(ref mut socket) = self.socket {
            match socket.write_message(tungstenite::Message::Text(subscribe_message)) {
                Ok(_) => (),
                Err(e) => self.on_error(&format!("Error sending subscribe message: {e}")),
            }
        }
    }

    fn on_close(&self) {
        println!("Connection closed.");
    }

    fn on_error(&self, error: &str) {
        println!("Error: {error}");
    }

    fn on_ping(&mut self, data: &[u8]) {
        println!("Ping: {:?}", data);
        if let Some(ref mut socket) = self.socket {
            let pong_message = tungstenite::Message::Pong(data.to_vec());
            println!("Sending pong: {:?}", pong_message);
            match socket.write_message(pong_message) {
                Ok(_) => (),
                Err(e) => self.on_error(&format!("Error sending pong: {e}")),
            }
        }
    }

    fn on_stop(&mut self) {
        println!("Stopping");
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.disconnect();
    }
}
