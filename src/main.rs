use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, Multipart, State},
    http::{StatusCode, Uri, HeaderMap},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, env, net::SocketAddr, path::PathBuf, collections::HashSet};
use tokio::sync::{broadcast, Mutex, RwLock};
use futures::{StreamExt, SinkExt};
use rand::Rng;
use tokio::fs;
use std::time::{SystemTime, UNIX_EPOCH, Duration, Instant};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
enum WsFrame {
    Assign { id: String, color: String },
    Msg { from: String, text: String, color: String, to: Option<String> },
    Upload { from: String, url: String, filename: String, to: Option<String> },
    System { text: String },
    Help { commands: Vec<String> },
    Whisper { from: String, to: String, text: String, color: String },
    Blocked { from: String },
    Warn { text: String },
    Banned { reason: String },
}

#[derive(Clone)]
struct UserActivity {
    pub message_count: usize,
    pub last_message_time: Instant,
    pub warnings: usize,
    pub is_banned: bool,
    pub ban_until: Option<Instant>,
}

type ChatHistory = Arc<tokio::sync::Mutex<Vec<String>>>;
type BlockList = Arc<tokio::sync::Mutex<HashSet<String>>>;
type UserActivities = Arc<RwLock<HashMap<String, UserActivity>>>;

#[derive(Clone)]
struct AppState {
    tx: Arc<broadcast::Sender<String>>,
    history: ChatHistory,
    user_activities: UserActivities,
}

const MESSAGE_RATE_LIMIT: usize = 10;
const MESSAGE_COUNT_LIMIT: usize = 5;
const MAX_WARNINGS: usize = 1;
const BAN_DURATION: Duration = Duration::from_secs(300);

fn gen_id() -> String {
    let n: u16 = rand::random();
    format!("u{:04x}", n)
}

fn gen_color() -> String {
    let r: u8 = 100 + (rand::random::<u8>() % 156);
    let g: u8 = 100 + (rand::random::<u8>() % 156);
    let b: u8 = 100 + (rand::random::<u8>() % 156);
    format!("#{:02x}{:02x}{:02x}", r, g, b)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    fs::create_dir_all("static/uploads").await.unwrap();

    let (tx, _rx) = broadcast::channel::<String>(256);
    let tx = Arc::new(tx);
    let history: ChatHistory = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let user_activities: UserActivities = Arc::new(RwLock::new(HashMap::new()));
    
    let state = AppState { 
        tx: tx.clone(), 
        history: history.clone(),
        user_activities: user_activities.clone(),
    };

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/upload", post(upload_handler))
        .route("/uploads/:filename", get(serve_uploaded_file))
        .fallback(get(static_files))
        .with_state(state);

    let port: u16 = env::var("PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(10000);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("서버가 http://localhost:{} 에서 실행 중입니다", port);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn serve_uploaded_file(uri: Uri) -> impl IntoResponse {
    let filename = uri.path().trim_start_matches("/uploads/");
    let file_path = PathBuf::from("static/uploads").join(filename);
    
    if file_path.exists() && file_path.is_file() {
        match tokio::fs::read(&file_path).await {
            Ok(content) => {
                let mime_type = mime_guess::from_path(&file_path).first_or_octet_stream();
                let mut headers = HeaderMap::new();
                headers.insert("Content-Type", mime_type.to_string().parse().unwrap());
                headers.insert("Content-Disposition", format!("inline; filename=\"{}\"", filename).parse().unwrap());
                
                (StatusCode::OK, headers, content).into_response()
            }
            Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read file").into_response(),
        }
    } else {
        (StatusCode::NOT_FOUND, "File not found").into_response()
    }
}

async fn static_files(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');
    let file_path = PathBuf::from("static").join(path);
    
    if file_path.exists() && file_path.is_file() {
        match tokio::fs::read(&file_path).await {
            Ok(content) => {
                let mime_type = mime_guess::from_path(&file_path).first_or_octet_stream();
                (StatusCode::OK, [(axum::http::header::CONTENT_TYPE, mime_type.to_string())], content).into_response()
            }
            Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read file").into_response(),
        }
    } else {
        let index_path = PathBuf::from("static/index.html");
        if index_path.exists() {
            match tokio::fs::read(&index_path).await {
                Ok(content) => (StatusCode::OK, [(axum::http::header::CONTENT_TYPE, "text/html")], content).into_response(),
                Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read index.html").into_response(),
            }
        } else {
            (StatusCode::NOT_FOUND, "Not found").into_response()
        }
    }
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn check_spam(user_id: &str, state: &AppState) -> Result<(), String> {
    let mut activities = state.user_activities.write().await;
    
    let now = Instant::now();
    let activity = activities.entry(user_id.to_string()).or_insert(UserActivity {
        message_count: 0,
        last_message_time: now,
        warnings: 0,
        is_banned: false,
        ban_until: None,
    });

    if activity.is_banned {
        if let Some(ban_until) = activity.ban_until {
            if now < ban_until {
                let remaining = ban_until.duration_since(now).as_secs();
                return Err(format!("차단 상태입니다. {}초 후에 다시 시도해주세요.", remaining));
            } else {
                activity.is_banned = false;
                activity.ban_until = None;
                activity.message_count = 0;
                activity.warnings = 0;
            }
        }
    }

    if now.duration_since(activity.last_message_time) < Duration::from_secs(1) {
        activity.message_count += 1;
    } else {
        activity.message_count = 1;
    }

    activity.last_message_time = now;

    if activity.message_count > MESSAGE_COUNT_LIMIT {
        activity.warnings += 1;
        
        if activity.warnings >= MAX_WARNINGS {
            activity.is_banned = true;
            activity.ban_until = Some(now + BAN_DURATION);
            return Err("도배로 인해 5분간 차단되었습니다.".to_string());
        }
        
        return Err(format!("너무 빠르게 메시지를 보내고 있습니다. 경고: {}/{}", activity.warnings, MAX_WARNINGS));
    }

    Ok(())
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let id = gen_id();
    let color = gen_color();
    let assign = WsFrame::Assign { id: id.clone(), color: color.clone() };
    let (mut sender, mut receiver) = socket.split();

    let block_list: BlockList = Arc::new(tokio::sync::Mutex::new(HashSet::new()));

    {
        let history = state.history.lock().await;
        for msg in history.iter() {
            let _ = sender.send(Message::Text(msg.clone())).await;
        }
    }

    let _ = sender.send(Message::Text(serde_json::to_string(&assign).unwrap())).await;

    let mut rx = state.tx.subscribe();
    let tx_clone = state.tx.clone();
    let history_clone = state.history.clone();
    let id_clone = id.clone();
    let color_clone = color.clone();
    let block_list_clone = block_list.clone();
    let state_clone = state.clone();

    let sender_arc = Arc::new(Mutex::new(sender));
    let sender_for_send_task = sender_arc.clone();

    let send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if let Ok(frame) = serde_json::from_str::<WsFrame>(&msg) {
                let block_list = block_list_clone.lock().await;
                match &frame {
                    WsFrame::Msg { from, .. } | WsFrame::Whisper { from, .. } => {
                        if block_list.contains(from) {
                            continue;
                        }
                    }
                    _ => {}
                }
            }
            let mut sender = sender_for_send_task.lock().await;
            if sender.send(Message::Text(msg)).await.is_err() { break; }
        }
    });

    while let Some(Ok(message)) = receiver.next().await {
        match message {
            Message::Text(text) => {
                if let Err(warning) = check_spam(&id_clone, &state_clone).await {
                    let frame = WsFrame::Warn { text: warning };
                    if let Ok(json) = serde_json::to_string(&frame) {
                        let mut sender = sender_arc.lock().await;
                        let _ = sender.send(Message::Text(json)).await;
                    }
                    continue;
                }

                if text.trim() == "/help" {
                    let commands = vec![
                        "/help - 도움말 보기".to_string(),
                        "/w [대상] [메시지] - 귓속말 보내기".to_string(),
                        "/block [대상] - 사용자 차단".to_string(),
                        "/unblock [대상] - 사용자 차단 해제".to_string(),
                        "/upload - 파일 업로드".to_string(),
                        "주의: 도배 시 자동 차단됩니다".to_string(),
                    ];
                    let frame = WsFrame::Help { commands };
                    if let Ok(json) = serde_json::to_string(&frame) {
                        let mut sender = sender_arc.lock().await;
                        let _ = sender.send(Message::Text(json)).await;
                    }
                    continue;
                }

                if text.starts_with("/block ") {
                    let target = text.trim_start_matches("/block ").trim();
                    if !target.is_empty() {
                        let mut block_list = block_list.lock().await;
                        block_list.insert(target.to_string());
                        let frame = WsFrame::System { 
                            text: format!("{} 사용자를 차단했습니다", target) 
                        };
                        if let Ok(json) = serde_json::to_string(&frame) {
                            let mut sender = sender_arc.lock().await;
                            let _ = sender.send(Message::Text(json)).await;
                        }
                    }
                    continue;
                }

                if text.starts_with("/unblock ") {
                    let target = text.trim_start_matches("/unblock ").trim();
                    if !target.is_empty() {
                        let mut block_list = block_list.lock().await;
                        block_list.remove(target);
                        let frame = WsFrame::System { 
                            text: format!("{} 사용자 차단을 해제했습니다", target) 
                        };
                        if let Ok(json) = serde_json::to_string(&frame) {
                            let mut sender = sender_arc.lock().await;
                            let _ = sender.send(Message::Text(json)).await;
                        }
                    }
                    continue;
                }

                if text.starts_with("/w ") {
                    let content = text.trim_start_matches("/w ");
                    if let Some(space_idx) = content.find(' ') {
                        let target = content[..space_idx].trim();
                        let message_text = content[space_idx+1..].trim();
                        
                        if !target.is_empty() && !message_text.is_empty() {
                            let frame = WsFrame::Whisper {
                                from: id_clone.clone(),
                                to: target.to_string(),
                                text: message_text.to_string(),
                                color: color_clone.clone(),
                            };
                            if let Ok(json) = serde_json::to_string(&frame) {
                                let _ = tx_clone.send(json);
                            }
                        }
                    }
                    continue;
                }

                if text.starts_with("/upload ") {
                    let filename = text.trim_start_matches("/upload ").trim();
                    let frame = WsFrame::System { 
                        text: format!("파일 업로드를 위해 선택 버튼을 사용해주세요: {}", filename) 
                    };
                    if let Ok(json) = serde_json::to_string(&frame) {
                        let mut sender = sender_arc.lock().await;
                        let _ = sender.send(Message::Text(json)).await;
                    }
                    continue;
                }

                let frame = WsFrame::Msg {
                    from: id_clone.clone(),
                    text: text.clone(),
                    color: color_clone.clone(),
                    to: None,
                };

                if let Ok(json) = serde_json::to_string(&frame) {
                    {
                        let mut hist = history_clone.lock().await;
                        hist.push(json.clone());
                    }
                    let _ = tx_clone.send(json);
                }
            }
            Message::Binary(data) => {
                println!("Received binary data of length: {}", data.len());
            }
            _ => {}
        }
    }

    send_task.abort();
}

async fn upload_handler(State(state): State<AppState>, mut multipart: Multipart) -> impl IntoResponse {
    let mut uploaded_files = Vec::new();

    while let Some(field) = multipart.next_field().await.unwrap() {
        let filename = field.file_name().unwrap_or("unknown").to_string();
        let data = field.bytes().await.unwrap();
        
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        
        let extension = PathBuf::from(&filename)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "bin".to_string());
        
        let unique_filename = format!("{}_{}.{}", timestamp, rand::random::<u16>(), extension);
        
        let save_path = PathBuf::from("static/uploads").join(&unique_filename);
        fs::create_dir_all(save_path.parent().unwrap()).await.unwrap();
        
        if let Err(e) = fs::write(&save_path, &data).await {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("파일 저장 실패: {}", e)).into_response();
        }

        uploaded_files.push((filename, unique_filename));
    }

    for (original_name, saved_name) in uploaded_files {
        let frame = WsFrame::Upload {
            from: "system".to_string(),
            url: format!("/uploads/{}", saved_name),
            filename: original_name,
            to: None,
        };

        if let Ok(json) = serde_json::to_string(&frame) {
            let _ = state.tx.send(json);
        }
    }

    (StatusCode::OK, "파일 업로드 성공").into_response()
}