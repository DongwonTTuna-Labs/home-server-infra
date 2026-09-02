import { createServer, type Server } from "node:http";
import { pathToFileURL } from "node:url";
export interface FakeChatGpt {
  port: number;
  baseUrl(scenario: string): string;
  close(): Promise<void>;
}
export async function startFakeChatGpt(port = 0): Promise<FakeChatGpt> {
  const server = createServer((request, response) => {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");
    if (url.pathname === "/c/WEB:stale-root"
      && url.searchParams.get("scenario") === "root-redirect") {
      response.writeHead(302, { location: "/?scenario=root-redirect&redirected=1" }).end();
      return;
    }
    if (url.pathname === "/download/report.txt" || url.pathname === "/download/numbers.txt") {
      const filename = url.pathname.endsWith("numbers.txt") ? "numbers.txt" : "report.txt";
      response.writeHead(200, {
        "content-type": "text/plain",
        "content-disposition": `attachment; filename="${filename}"`,
      }).end(filename === "numbers.txt" ? "1\n2\n3\n" : "report from fake ChatGPT\n");
      return;
    }
    if (url.pathname === "/download/pack.zip") {
      // 실측: 직접 다운로드 버튼은 content-disposition 파일명을 제네릭 "download"로 준다.
      response.writeHead(200, {
        "content-type": "application/zip",
        "content-disposition": 'attachment; filename="download"',
      }).end(Buffer.from("PK\u0003\u0004fake-zip-body"));
      return;
    }
    if (url.pathname === "/download/archive.tar.gz") {
      response.writeHead(200, {
        "content-type": "application/gzip",
        "content-disposition": 'attachment; filename="archive.tar.gz"',
      }).end(Buffer.from("fake-tar-gz"));
      return;
    }
    response.writeHead(200, { "content-type": "text/html; charset=utf-8" }).end(PAGE);
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () => resolve());
  });
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("fake server did not bind TCP");
  return {
    port: address.port,
    baseUrl(scenario: string) {
      return `http://127.0.0.1:${address.port}/?scenario=${encodeURIComponent(scenario)}`;
    },
    close: () => closeServer(server),
  };
}
function closeServer(server: Server): Promise<void> {
  return new Promise((resolve, reject) => {
    server.close((error) => error ? reject(error) : resolve());
  });
}
const PAGE = String.raw`<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Fake ChatGPT</title>
  <style>
    body { font-family: sans-serif; margin: 24px; }
    button, a { min-width: 32px; min-height: 24px; }
    #prompt-textarea { min-height: 48px; min-width: 480px; border: 1px solid #888; }
    [data-message-author-role] { min-height: 24px; margin: 8px 0; white-space: pre-wrap; }
    .chip { display: inline-flex; gap: 6px; min-width: 80px; min-height: 24px; }
    .action { display: inline-block; width: 32px; height: 24px; }
  </style>
</head>
<body><main id="app"></main>
<script>
(() => {
  const query = new URL(location.href).searchParams;
  const scenario = query.get('scenario') || 'happy';
  const tabId = typeof crypto.randomUUID === 'function'
    ? crypto.randomUUID()
    : 'local-' + Date.now() + '-' + Math.random().toString(16).slice(2);
  if (scenario === 'root-redirect' && query.get('redirected') === '1') {
    localStorage.setItem('gwp-root-redirected', String(Date.now()));
  }
  const app = document.getElementById('app');
  if (scenario === 'login-wall') {
    app.innerHTML = '<section data-testid="login-wall"><button>Log in</button><button>Sign up</button></section>';
    return;
  }
  if (scenario === 'rate-limit') {
    app.innerHTML = '<section role="alert" data-testid="rate-limit">Too many requests. Try again later.</section>';
    return;
  }
  const initialIntelligence = scenario === 'slow' ? 'Pro' : 'Instant';
  const intelligenceLabels = scenario === 'model-missing'
    ? ['Instant', 'Medium', 'High', 'Extra High']
    : ['Instant', 'Medium', 'High', 'Extra High', 'Pro'];
  const intelligenceMarkup = (label) => label === 'Instant'
    ? '<span>Instant</span><br><span>5.5</span>'
    : '<span>' + label + '</span>';
  const intelligenceRadios = intelligenceLabels.map((label) => (
    '<button type="button" role="menuitemradio" data-intelligence="' + label + '" aria-checked="' +
      String(label === initialIntelligence) + '">' + intelligenceMarkup(label) + '</button>'
  )).join('');
  app.innerHTML =
    '<section id="conversation"></section>' +
    '<form id="composer-form" onsubmit="return false">' +
    '<button type="button" class="__composer-pill" id="intelligence-pill" aria-haspopup="menu" ' +
      'aria-expanded="false" data-open-count="0">' + intelligenceMarkup(initialIntelligence) +
      '<svg aria-hidden="true"></svg></button>' +
    '<div id="chips"></div><input type="file" multiple aria-label="Attach files">' +
    '<div id="prompt-textarea" contenteditable="true" role="textbox"></div>' +
    '<button type="button" data-testid="send-button" aria-label="Send prompt">Send</button></form>' +
    '<div data-radix-popper-content-wrapper id="intelligence-popper" hidden>' +
      '<div data-testid="composer-intelligence-picker-content" aria-label="Intelligence">' +
        '<div role="menu" aria-label="Intelligence">' + intelligenceRadios +
          '<button type="button" role="menuitem">GPT-5.6 Sol</button>' +
        '</div>' +
      '</div>' +
    '</div>';
  const intelligencePill = document.getElementById('intelligence-pill');
  const intelligencePopper = document.getElementById('intelligence-popper');
  const intelligenceItems = Array.from(app.querySelectorAll('[role="menu"] [role="menuitemradio"]'));
  intelligencePill.addEventListener('click', () => {
    intelligencePill.dataset.openCount = String(Number(intelligencePill.dataset.openCount) + 1);
    intelligencePill.setAttribute('aria-expanded', 'true');
    setTimeout(() => { intelligencePopper.hidden = false; }, 100);
  });
  for (const item of intelligenceItems) {
    item.addEventListener('click', () => {
      for (const candidate of intelligenceItems) candidate.setAttribute('aria-checked', 'false');
      if (scenario !== 'model-check-fails') item.setAttribute('aria-checked', 'true');
      const label = item.dataset.intelligence;
      intelligencePill.innerHTML = intelligenceMarkup(label) + '<svg aria-hidden="true"></svg>';
      intelligencePill.setAttribute('aria-expanded', 'false');
      intelligencePopper.hidden = true;
    });
  }
  const input = app.querySelector('input[type=file]');
  const chips = document.getElementById('chips');
  input.addEventListener('change', () => {
    chips.replaceChildren();
    const counts = new Map();
    for (const file of input.files) {
      const count = counts.get(file.name) || 0;
      counts.set(file.name, count + 1);
      const name = count === 0 ? file.name : duplicateName(file.name, count);
      const root = document.createElement('div');
      root.className = 'chip';
      const label = document.createElement('span');
      label.textContent = name;
      const remove = document.createElement('button');
      remove.type = 'button';
      remove.setAttribute('aria-label', 'Remove file: ' + name);
      remove.textContent = 'Remove';
      root.append(label, remove);
      chips.append(root);
    }
  });
  let sequence = 0;
  app.querySelector('[data-testid="send-button"]').addEventListener('click', () => {
    const prompt = document.getElementById('prompt-textarea').innerText.trim();
    let renderedPrompt = scenario === 'markdown-normalization'
      ? prompt.split('\n').flatMap((raw) => {
        const line = raw.trim();
        const fence = line.match(/^\x60{3}([A-Za-z0-9_.+-]+)?$/);
        return fence ? (fence[1] ? [fence[1]] : []) : [line];
      }).join('\n')
      : prompt;
    if (scenario === 'large-prompt-drift') {
      // 실 UI가 대형 user 턴을 원문과 다르게 렌더하는 상황(중간 축약)을 재현한다.
      const middle = Math.floor(prompt.length / 2);
      renderedPrompt = prompt.slice(0, middle) + prompt.slice(middle + Math.floor(prompt.length * 0.03));
    }
    if (scenario === 'markdown-drift') {
      // user 턴 마크다운 렌더로 문법 문자가 문서 전체에서 소실되는 실측 모드(2026-07-29).
      renderedPrompt = prompt.replace(/\*\*/g, '').replace(/\x60/g, '');
    }
    const conversation = document.getElementById('conversation');
    const user = document.createElement('div');
    user.setAttribute('data-message-author-role', 'user');
    user.setAttribute('data-message-id', 'user-' + tabId + '-' + (++sequence));
    const attachmentPrefix = scenario === 'attachments'
      ? Array.from(chips.querySelectorAll('span')).map((node) => node.textContent + '\nFile').join('\n')
      : '';
    user.textContent = attachmentPrefix ? attachmentPrefix + '\n' + renderedPrompt : renderedPrompt;
    const assistant = document.createElement('div');
    assistant.setAttribute('data-message-author-role', 'assistant');
    assistant.setAttribute('data-message-id', scenario === 'assistant-id-rebind'
      ? 'request-placeholder-request-WEB:fake-' + tabId + '-' + sequence + '-0'
      : 'assistant-' + tabId + '-' + sequence);
    assistant.dataset.prompt = prompt;
    // 실 UI처럼 assistant 메시지를 article로 감싼다 — 액션 바(copy)는 메시지 노드의
    // 형제로 붙는다 (2026-07-27 실측 구조).
    const assistantArticle = document.createElement('article');
    assistantArticle.append(assistant);
    conversation.append(user);
    if (scenario !== 'confirmation-miss') conversation.append(assistantArticle);
    const conversationPath = scenario === 'url-rebind'
      ? '/c/WEB:fake-' + tabId + '-' + sequence
      : '/c/fake-' + tabId + '-' + sequence;
    history.pushState({}, '', conversationPath + '?scenario=' + encodeURIComponent(scenario));
    if (scenario === 'confirmation-miss') return;
    if (scenario === 'url-rebind') {
      setTimeout(() => {
        history.replaceState({}, '', '/c/final-' + tabId + '-' + sequence + '?scenario=' + encodeURIComponent(scenario));
      }, 500);
    }
    const stop = document.createElement('button');
    stop.type = 'button';
    stop.setAttribute('aria-label', 'Stop generating');
    stop.textContent = 'Stop';
    app.append(stop);
    if (scenario === 'slow') {
      assistant.textContent = 'still generating';
      return;
    }
    if (scenario === 'post-stream-gap') {
      setTimeout(() => stop.remove(), 250);
      setTimeout(() => finish(assistant, scenario), 1750);
      return;
    }
    setTimeout(() => { assistant.textContent = 'fake'; }, 100);
    setTimeout(() => {
      stop.remove();
      finish(assistant, scenario);
    }, 300);
  });
  function finish(assistant, currentScenario) {
    assistant.textContent = currentScenario === 'artifacts-delayed'
      ? 'numbers.txt.'
      : currentScenario === 'artifacts-empty'
        ? ''
      : currentScenario === 'artifacts-no-hint'
        ? 'ordinary answer'
        : currentScenario === 'artifacts' || currentScenario === 'artifacts-inline'
          ? 'artifact answer'
      : currentScenario === 'artifacts-direct'
        ? 'RouteFork_pack.zip 다운로드'
      : currentScenario === 'multi-tab'
        ? 'answer for ' + assistant.dataset.prompt
        : 'fake answer';
    const copy = document.createElement('button');
    copy.type = 'button';
    copy.className = 'action';
    copy.dataset.testid = 'copy-turn-action-button';
    copy.setAttribute('aria-label', 'Copy response');
    const actionBar = document.createElement('div');
    actionBar.append(copy);
    assistant.parentElement.append(actionBar);
    if (currentScenario === 'assistant-id-rebind') {
      assistant.setAttribute(
        'data-message-id',
        'e6888530-7b2b-4c3d-9e5f-' + String(sequence).padStart(12, '0'),
      );
    }
    if (currentScenario === 'artifacts') {
      assistant.append(fileEntity('/download/report.txt', 'report.txt'));
      assistant.append(fileEntity('/download/archive.tar.gz', 'archive.tar.gz'));
    }
    if (currentScenario === 'artifacts-inline') {
      assistant.append(download('/download/report.txt', 'report.txt'));
      assistant.append(download('/download/archive.tar.gz', 'archive.tar.gz'));
    }
    if (currentScenario === 'artifacts-direct') {
      assistant.append(directDownload('/download/pack.zip', 'RouteFork_pack.zip'));
    }
    if (currentScenario === 'artifacts-delayed' || currentScenario === 'artifacts-empty') {
      setTimeout(() => assistant.append(fileEntity('/download/numbers.txt', 'numbers.txt', true)), 4000);
    }
    if (currentScenario === 'artifacts-no-hint') {
      setTimeout(() => assistant.append(fileEntity('/download/report.txt', 'report.txt')), 5000);
    }
  }
  function fileEntity(href, filename, polluting = false) {
    const entity = document.createElement('button');
    entity.type = 'button';
    entity.className = 'behavior-btn';
    entity.setAttribute('aria-label', filename);
    entity.textContent = filename;
    entity.addEventListener('click', () => {
      const panel = document.createElement('section');
      panel.dataset.filePreview = filename;
      const downloadButton = document.createElement('button');
      downloadButton.type = 'button';
      downloadButton.setAttribute('aria-label', 'Download');
      downloadButton.textContent = 'Download';
      downloadButton.addEventListener('click', () => {
        const link = document.createElement('a');
        link.href = href;
        link.download = filename;
        document.body.append(link);
        link.click();
        link.remove();
      });
      panel.append(downloadButton);
      app.append(panel);
    });
    if (!polluting) return entity;
    const block = document.createElement('div');
    block.append('Download', entity, '.');
    return block;
  }
  function directDownload(href, filename) {
    // 실측: 생성 파일(zip 등)은 텍스트가 '<파일명> 다운로드'인 버튼으로 렌더되고,
    // 클릭하면 미리보기 패널 없이 곧바로 다운로드가 시작된다. aria-label은 없다.
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'behavior-btn';
    button.textContent = filename + ' 다운로드';
    button.addEventListener('click', () => {
      const link = document.createElement('a');
      link.href = href;
      document.body.append(link);
      link.click();
      link.remove();
    });
    return button;
  }
  function download(href, filename) {
    const link = document.createElement('a');
    link.className = 'action';
    link.href = href;
    link.download = filename;
    link.setAttribute('aria-label', 'Download ' + filename);
    return link;
  }
  function duplicateName(name, count) {
    const index = name.lastIndexOf('.');
    return index > 0
      ? name.slice(0, index) + ' (' + count + ')' + name.slice(index)
      : name + ' (' + count + ')';
  }
})();
</script></body></html>`;
async function cli(): Promise<void> {
  const index = process.argv.indexOf("--port");
  const port = index >= 0 ? Number(process.argv[index + 1]) : 0;
  const fake = await startFakeChatGpt(port);
  process.stdout.write(`${fake.port}\n`);
}
if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  cli().catch((error) => {
    process.stderr.write(`${String(error)}\n`);
    process.exitCode = 1;
  });
}
