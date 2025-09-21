const chat = document.getElementById('chat');
const messageInput = document.getElementById('messageInput');
const sendButton = document.getElementById('sendButton');
const connectionStatus = document.getElementById('connectionStatus');
const fileInput = document.getElementById('fileInput');
const uploadProgress = document.getElementById('uploadProgress');

const protocol = location.protocol === 'https:' ? 'wss' : 'ws';
const wsUrl = protocol + '://' + location.host + '/ws';
const ws = new WebSocket(wsUrl);

let myId = null;
let myColor = '#888';

function updateConnectionStatus(status) {
  connectionStatus.className = `connection-status ${status}`;
  connectionStatus.textContent = status === 'connected' ? '온라인' : '연결 끊김';
}

function showUploadProgress(show) {
  uploadProgress.style.display = show ? 'block' : 'none';
}

function getInitials(username) {
  return username.slice(0, 2).toUpperCase();
}

function formatTime() {
  const now = new Date();
  return now.toLocaleTimeString('ko-KR', { hour: '2-digit', minute: '2-digit' });
}

function appendMessage(username, text, color, isSystem = false, isUpload = false, isWhisper = false, isHelp = false, fileData = null, commands = []) {
  const messageEl = document.createElement('div');
  let messageClass = 'message';
  if (isSystem) messageClass += ' system';
  if (isUpload) messageClass += ' upload';
  if (isWhisper) messageClass += ' whisper';
  if (isHelp) messageClass += ' help';
  
  messageEl.className = messageClass;
  messageEl.style.setProperty('--user-color', color);
  
  const time = formatTime();
  
  if (isSystem) {
    messageEl.innerHTML = `
      <div class="avatar" style="background: #34d399;">SYS</div>
      <div class="message-content">
        <div class="username" style="color: #34d399;">시스템</div>
        <div class="message-text">${text}</div>
        <div class="timestamp">${time}</div>
      </div>
    `;
  } else if (isHelp) {
    messageEl.innerHTML = `
      <div class="avatar" style="background: #8b5cf6;">?</div>
      <div class="message-content">
        <div class="username" style="color: #8b5cf6;">도움말</div>
        <div class="message-text">${text}</div>
        <div class="help-commands">
          ${commands.map(cmd => `<div class="help-command">${cmd}</div>`).join('')}
        </div>
        <div class="timestamp">${time}</div>
      </div>
    `;
  } else if (isUpload && fileData) {
    messageEl.innerHTML = `
      <div class="avatar">${getInitials(username)}</div>
      <div class="message-content">
        <div class="username">${username}</div>
        <div class="message-text">파일을 공유했습니다</div>
        <div class="upload-file">
          <span class="upload-icon">📎</span>
          <div>
            <a href="${fileData.url}" class="upload-link" target="_blank" download="${fileData.filename}">
              ${fileData.filename}
            </a>
            <div class="upload-filename">${formatFileSize(fileData.size)}</div>
          </div>
        </div>
        <div class="timestamp">${time}</div>
      </div>
    `;
  } else if (isWhisper) {
    messageEl.innerHTML = `
      <div class="avatar">${getInitials(username)}</div>
      <div class="message-content">
        <div class="username">${username} (귓속말)</div>
        <div class="message-text">${text}</div>
        <div class="timestamp">${time}</div>
      </div>
    `;
  } else {
    messageEl.innerHTML = `
      <div class="avatar">${getInitials(username)}</div>
      <div class="message-content">
        <div class="username">${username}</div>
        <div class="message-text">${text}</div>
        <div class="timestamp">${time}</div>
      </div>
    `;
  }
  
  chat.appendChild(messageEl);
  chat.scrollTop = chat.scrollHeight;
}

function formatFileSize(bytes) {
  if (bytes === 0) return '0 Bytes';
  const k = 1024;
  const sizes = ['Bytes', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
}

ws.addEventListener('open', () => {
  updateConnectionStatus('connected');
});

ws.addEventListener('close', () => {
  updateConnectionStatus('disconnected');
});

ws.addEventListener('error', () => {
  updateConnectionStatus('disconnected');
});

ws.addEventListener('message', (e) => {
  try {
    const data = JSON.parse(e.data);
    
    if (data.type === 'assign') {
      myId = data.id;
      myColor = data.color;
      appendMessage('', `환영합니다! 당신의 ID는 ${myId}입니다.`, '#34d399', true);
      appendMessage('', `도움말: /help`, '#34d399', true);
    } else if (data.type === 'msg') {
      appendMessage(data.from, data.text, data.color);
    } else if (data.type === 'upload') {
      appendMessage(data.from, '', data.color, false, true, false, false, {
        url: data.url,
        filename: data.filename,
        size: 0
      });
    } else if (data.type === 'system') {
      appendMessage('', data.text, '#34d399', true);
    } else if (data.type === 'help') {
      appendMessage('', '사용 가능한 명령어:', '#8b5cf6', false, false, false, true, null, data.commands);
    } else if (data.type === 'whisper') {
      appendMessage(data.from, data.text, data.color, false, false, true);
    } else if (data.type === 'blocked') {
      appendMessage('', `${data.from} 사용자로부터 메시지를 차단했습니다`, '#ef4444', true);
    }
  } catch (error) {
    console.error('메시지 파싱 오류:', error);
  }
});

async function uploadFile(file) {
  const formData = new FormData();
  formData.append('file', file);
  
  showUploadProgress(true);
  
  try {
    const response = await fetch('/upload', {
      method: 'POST',
      body: formData
    });
    
    if (response.ok) {
      console.log('파일 업로드 성공');
    } else {
      console.error('파일 업로드 실패');
      appendMessage('', '파일 업로드에 실패했습니다', '#ef4444', true);
    }
  } catch (error) {
    console.error('업로드 오류:', error);
    appendMessage('', '파일 업로드 중 오류가 발생했습니다', '#ef4444', true);
  } finally {
    showUploadProgress(false);
  }
}

fileInput.addEventListener('change', (e) => {
  const files = e.target.files;
  if (files.length > 0) {
    for (let file of files) {
      if (file.size > 10 * 1024 * 1024) {
        appendMessage('', '파일 크기는 10MB를 초과할 수 없습니다', '#ef4444', true);
        continue;
      }
      uploadFile(file);
    }
    fileInput.value = '';
  }
});

function sendMessage() {
  const text = messageInput.value.trim();
  if (!text || ws.readyState !== WebSocket.OPEN) return;
  
  ws.send(text);
  messageInput.value = '';
}

sendButton.addEventListener('click', sendMessage);
messageInput.addEventListener('keypress', (e) => {
  if (e.key === 'Enter') {
    e.preventDefault();
    sendMessage();
  }
});

messageInput.focus();