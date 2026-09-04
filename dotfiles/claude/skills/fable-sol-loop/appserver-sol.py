#!/usr/bin/env python3
"""codex app-server 데몬 경유 sol 실행 러너 (fable-sol-loop 스킬 동봉).

`codex exec` 대신 로컬 app-server 데몬(unix 소켓, WebSocket JSON-RPC v2)에
thread/turn을 만들어 실행한다. 장점:
  - Codex 앱(데스크톱/모바일)의 해당 cwd 워크스페이스 목록에 대화가 실시간으로 보인다
    (`codex exec`는 source=exec라 앱 기본 목록에서 숨겨진다).
  - turn은 데몬에 상주하므로 이 러너/터미널이 죽어도 계속 실행된다.
    회수는 `read`/`wait`로 언제든 가능.

usage:
  appserver-sol.py start  --cwd DIR --prompt-file F [--model M] [--effort E]
                          [--out OUT] [--rc RC] [--no-wait]
  appserver-sol.py resume --thread ID --prompt-file F [--model M] [--effort E]
                          [--out OUT] [--rc RC] [--no-wait]
  appserver-sol.py wait   --thread ID [--out OUT] [--rc RC]
  appserver-sol.py read   --thread ID [--out OUT]
  appserver-sol.py status --thread ID

start/resume는 turn 시작 직후 `THREAD_ID=...`를 출력·flush하므로, 백그라운드로
띄웠어도 로그에서 즉시 id를 회수해 state.md에 기록할 수 있다.
"""
import argparse
import base64
import json
import os
import socket
import struct
import sys
import time

SOCK = os.path.expanduser('~/.codex/app-server-control/app-server-control.sock')
CLIENT_INFO = {'name': 'fable_sol_loop', 'title': 'Fable orchestrator', 'version': '0.145.0'}
POLL_INTERVAL = 30


class WS:
    def __init__(self, path=SOCK):
        self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.s.settimeout(10)
        self.s.connect(path)
        key = base64.b64encode(os.urandom(16)).decode()
        req = (f"GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n"
               f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
               f"Sec-WebSocket-Version: 13\r\n\r\n")
        self.s.sendall(req.encode())
        buf = b''
        while b'\r\n\r\n' not in buf:
            buf += self.s.recv(4096)
        head, _, rest = buf.partition(b'\r\n\r\n')
        if b'101' not in head.split(b'\r\n')[0]:
            raise ConnectionError(f'handshake failed: {head[:200]!r}')
        self.rbuf = rest

    def send_text(self, text):
        payload = text.encode()
        mask = os.urandom(4)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        n = len(payload)
        if n < 126:
            hdr = struct.pack('!BB', 0x81, 0x80 | n)
        elif n < 65536:
            hdr = struct.pack('!BBH', 0x81, 0x80 | 126, n)
        else:
            hdr = struct.pack('!BBQ', 0x81, 0x80 | 127, n)
        self.s.sendall(hdr + mask + masked)

    def _read(self, n):
        while len(self.rbuf) < n:
            chunk = self.s.recv(65536)
            if not chunk:
                raise ConnectionError('socket closed')
            self.rbuf += chunk
        out, self.rbuf = self.rbuf[:n], self.rbuf[n:]
        return out

    def recv_frame(self, timeout=30):
        self.s.settimeout(timeout)
        b1, b2 = self._read(2)
        opcode = b1 & 0x0F
        masked = b2 & 0x80
        n = b2 & 0x7F
        if n == 126:
            n = struct.unpack('!H', self._read(2))[0]
        elif n == 127:
            n = struct.unpack('!Q', self._read(8))[0]
        mask = self._read(4) if masked else None
        payload = self._read(n)
        if mask:
            payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        if opcode == 0x9:  # ping -> pong
            m = os.urandom(4)
            self.s.sendall(struct.pack('!BB', 0x8A, 0x80 | len(payload)) + m +
                           bytes(b ^ m[i % 4] for i, b in enumerate(payload)))
            return self.recv_frame(timeout)
        if opcode == 0x8:
            raise ConnectionError('server sent close frame')
        return payload.decode('utf-8', 'replace')


class RPC:
    def __init__(self):
        self.ws = WS()
        self.next_id = 1
        self.request('initialize', {'clientInfo': CLIENT_INFO})

    def request(self, method, params, timeout=120):
        rid = self.next_id
        self.next_id += 1
        self.ws.send_text(json.dumps({'jsonrpc': '2.0', 'id': rid, 'method': method, 'params': params}))
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                msg = json.loads(self.ws.recv_frame(timeout=max(1, deadline - time.time())))
            except socket.timeout:
                continue
            if msg.get('id') == rid and ('result' in msg or 'error' in msg):
                if 'error' in msg:
                    raise RuntimeError(f'{method}: {json.dumps(msg["error"], ensure_ascii=False)[:500]}')
                return msg['result']
        raise TimeoutError(f'rpc timeout: {method}')

    def next_notification(self, timeout=60):
        msg = json.loads(self.ws.recv_frame(timeout=timeout))
        return msg


def log(msg):
    print(f'[{time.strftime("%H:%M:%S")}] {msg}', flush=True)


def read_thread(rpc, thread_id):
    return rpc.request('thread/read', {'threadId': thread_id, 'includeTurns': True}, timeout=180)


def last_turn(thread_data):
    turns = thread_data.get('thread', {}).get('turns') or []
    return turns[-1] if turns else None


def final_agent_text(turn):
    if not turn:
        return None
    texts = [i.get('text', '') for i in turn.get('items', []) if i.get('type') == 'agentMessage']
    return texts[-1] if texts else None


def write_outputs(turn, out_path, rc_path):
    text = final_agent_text(turn)
    status = turn.get('status') if turn else 'unknown'
    err = turn.get('error') if turn else None
    if out_path and text is not None:
        with open(out_path, 'w') as f:
            f.write(text)
        log(f'final message -> {out_path} ({len(text)} bytes)')
    if rc_path:
        rc = 0 if status == 'completed' and not err else 1
        with open(rc_path, 'w') as f:
            f.write(f'exit={rc} status={status}' + (f' error={json.dumps(err, ensure_ascii=False)[:300]}' if err else '') + '\n')
    return status, err


def wait_for_turn(thread_id, turn_id, rpc=None, out=None, rc=None):
    """Stream notifications until the turn ends; on connection loss, fall back to
    polling thread/read (the turn keeps running inside the daemon)."""
    try:
        probe = last_turn(read_thread(rpc or RPC(), thread_id))
        if probe and (not turn_id or probe.get('id') == turn_id) and probe.get('status') in ('completed', 'failed'):
            status, err = write_outputs(probe, out, rc)
            log(f'DONE status={status}' + (f' error={err}' if err else ''))
            return 0 if status == 'completed' and not err else 1
    except Exception as e:
        log(f'initial probe failed ({e}); falling back to streaming')
    while True:
        try:
            if rpc is None:
                rpc = RPC()
            msg = rpc.next_notification(timeout=120)
            m = msg.get('method', '')
            if m == 'turn/completed' or m == 'turn/failed':
                p = msg.get('params', {})
                if p.get('threadId') == thread_id and (not turn_id or p.get('turn', {}).get('id') == turn_id):
                    log(f'notification: {m}')
                    break
            elif m == 'thread/status/changed':
                pass
        except socket.timeout:
            # long silence is normal while the model works; verify via read
            try:
                turn = last_turn(read_thread(rpc, thread_id))
                if turn and (not turn_id or turn.get('id') == turn_id) and turn.get('status') in ('completed', 'failed'):
                    log(f'poll: turn status={turn.get("status")}')
                    break
                log(f'poll: turn status={turn.get("status") if turn else "none"} — still running')
            except Exception as e:
                log(f'poll error ({e}); reconnecting')
                rpc = None
        except (ConnectionError, RuntimeError, OSError) as e:
            log(f'connection lost ({e}); reconnecting in {POLL_INTERVAL}s (turn keeps running in daemon)')
            rpc = None
            time.sleep(POLL_INTERVAL)
    final = last_turn(read_thread(rpc or RPC(), thread_id))
    status, err = write_outputs(final, out, rc)
    log(f'DONE status={status}' + (f' error={err}' if err else ''))
    return 0 if status == 'completed' and not err else 1


def start_turn(rpc, thread_id, prompt, model, effort):
    params = {'threadId': thread_id, 'input': [{'type': 'text', 'text': prompt}]}
    if model:
        params['model'] = model
    if effort:
        params['effort'] = effort
    return rpc.request('turn/start', params, timeout=180)


def request_loaded(rpc, method, params, thread_id):
    """Call a method that needs the thread loaded in the daemon; auto thread/resume once."""
    try:
        return rpc.request(method, params, timeout=60)
    except RuntimeError as e:
        if 'thread not found' not in str(e):
            raise
        rpc.request('thread/resume', {'threadId': thread_id}, timeout=120)
        return rpc.request(method, params, timeout=60)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('cmd', choices=['start', 'resume', 'wait', 'read', 'status', 'steer', 'interrupt'])
    ap.add_argument('--cwd')
    ap.add_argument('--thread')
    ap.add_argument('--prompt-file')
    ap.add_argument('--model', default='gpt-5.6-sol')
    ap.add_argument('--effort', default='ultra')
    ap.add_argument('--out')
    ap.add_argument('--rc')
    ap.add_argument('--no-wait', action='store_true')
    a = ap.parse_args()

    if a.cmd in ('start', 'resume'):
        prompt = open(a.prompt_file).read() if a.prompt_file else sys.stdin.read()
        rpc = RPC()
        if a.cmd == 'start':
            if not a.cwd:
                sys.exit('--cwd required for start')
            th = rpc.request('thread/start', {
                'cwd': a.cwd,
                'model': a.model,
                'approvalPolicy': 'never',
                'sandbox': 'danger-full-access',
                'config': {'model_reasoning_effort': a.effort},
            }, timeout=120)
            thread_id = th['thread']['id']
        else:
            if not a.thread:
                sys.exit('--thread required for resume')
            rpc.request('thread/resume', {'threadId': a.thread}, timeout=120)
            thread_id = a.thread
        turn = start_turn(rpc, thread_id, prompt, a.model, a.effort)
        turn_id = turn['turn']['id']
        print(f'THREAD_ID={thread_id}', flush=True)
        print(f'TURN_ID={turn_id}', flush=True)
        if a.no_wait:
            return 0
        return wait_for_turn(thread_id, turn_id, rpc=rpc, out=a.out, rc=a.rc)

    if not a.thread:
        sys.exit('--thread required')
    if a.cmd == 'wait':
        return wait_for_turn(a.thread, None, rpc=None, out=a.out, rc=a.rc)
    if a.cmd in ('steer', 'interrupt'):
        rpc = RPC()
        turn = last_turn(read_thread(rpc, a.thread))
        if not turn or turn.get('status') not in ('inProgress', 'queued'):
            sys.exit(f'{a.cmd}: no active turn (last turn status={turn.get("status") if turn else None})')
        turn_id = turn.get('id')
        if a.cmd == 'steer':
            # Inject a mid-turn supervision message into the ACTIVE turn (no abort).
            text = open(a.prompt_file).read() if a.prompt_file else sys.stdin.read()
            if not text.strip():
                sys.exit('steer: empty message')
            res = request_loaded(rpc, 'turn/steer', {
                'threadId': a.thread, 'expectedTurnId': turn_id,
                'input': [{'type': 'text', 'text': text}],
            }, a.thread)
        else:
            # Abort the active turn (thread survives; follow with `resume` + corrective prompt).
            res = request_loaded(rpc, 'turn/interrupt',
                                 {'threadId': a.thread, 'turnId': turn_id}, a.thread)
        log(f'{a.cmd} accepted (turn {turn_id}): {json.dumps(res, ensure_ascii=False)[:300]}')
        return 0
    rpc = RPC()
    data = read_thread(rpc, a.thread)
    turn = last_turn(data)
    if a.cmd == 'status':
        t = data.get('thread', {})
        print(json.dumps({
            'threadId': t.get('id'), 'status': t.get('status'), 'cwd': t.get('cwd'),
            'turns': len(t.get('turns') or []),
            'lastTurnStatus': turn.get('status') if turn else None,
        }, ensure_ascii=False))
        return 0
    # read
    text = final_agent_text(turn)
    if a.out and text is not None:
        with open(a.out, 'w') as f:
            f.write(text)
        print(f'wrote {a.out} ({len(text)} bytes)')
    else:
        print(text if text is not None else '(no agent message yet)')
    return 0


if __name__ == '__main__':
    sys.exit(main())
