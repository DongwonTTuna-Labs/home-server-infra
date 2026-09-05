import { createServer, type Server } from "node:http";
import { pathToFileURL } from "node:url";
import sharp from "sharp";
export interface FakeChatGpt {
  port: number;
  baseUrl(scenario: string): string;
  close(): Promise<void>;
}
export async function startFakeChatGpt(port = 0): Promise<FakeChatGpt> {
  const imageFiles = await Promise.all(["#993322", "#229933", "#332299", "#aa8800", "#0088aa"].map((background) => (
    sharp({ create: { width: 512, height: 384, channels: 3, background } }).png().toBuffer()
  )));
  const server = createServer((request, response) => {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");
    const imageIndex = url.pathname.match(/^\/(preview|download)\/image-([0-4])\.png$/u);
    if (imageIndex) {
      response.writeHead(200, { "content-type": "image/png",
        ...(imageIndex[1] === "download" ? { "content-disposition": 'attachment; filename="image.png"' } : {}) }).end(imageFiles[Number(imageIndex[2])]);
      return;
    }
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
  const gpt6 = scenario.startsWith('gpt6');
  const versionConfirmation = scenario.startsWith('gpt6-model-');
  const powerMenu = scenario.startsWith('gpt6-power-menu') || versionConfirmation;
  const legacyModel = scenario === 'gpt6-legacy-model' || versionConfirmation;
  const initialIntelligence = legacyModel || scenario === 'slow' ? 'Pro' : powerMenu ? 'Extra High' : 'Instant';
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
  // 2026-09 GPT-6 UI 재현: 알약 "6\nPro", 피커 본문에 Select model(라디오 Latest/GPT-5.6 Sol/GPT-5.5)
  // + 생각 강도 슬라이더(0..4, 4=Pro). 슬라이더 값은 즉시 적용되고 Escape로 닫는다.
  const POWER_LEVELS = powerMenu ? ['Instant', 'Medium', 'High', 'Extra High', 'Pro'] : ['Instant', 'Light', 'Standard', 'Extended', 'Pro'];
  const gpt6PillMarkup = (power) => '<span>6</span><br><span>' + power + '</span>';
  const gpt6Versions = ['Latest', 'GPT-5.6 Sol', 'GPT-5.5'];
  const gpt6InitialVersion = legacyModel ? 'GPT-5.5' : 'Latest';
  const gpt6InitialPower = POWER_LEVELS.indexOf(initialIntelligence);
  const gpt6Popper =
    '<div data-radix-popper-content-wrapper id="intelligence-popper" hidden>' +
      '<div role="menu" tabindex="-1">' +
        '<div role="group" data-testid="composer-intelligence-picker-content">' +
          '<div role="group"><div role="menuitem" aria-label="Select model" aria-expanded="false" tabindex="0" id="model-select">' +
            '<span>6</span><span id="model-select-power">' + initialIntelligence + '</span></div></div>' +
          '<div data-testid="composer-model-picker-slider-simple-view">' +
            '<div role="menuitem" aria-label="Power" tabindex="0" id="power-control">' +
              '<div role="slider" id="power-slider" tabindex="' + (powerMenu ? '-1' : '0') + '" style="display:' + (powerMenu ? 'none' : 'inline-block') + ';width:160px;height:14px;background:#ccc" aria-valuemin="0" aria-valuemax="4" aria-valuenow="' + gpt6InitialPower + '"></div>' +
              '<span id="power-status">' + initialIntelligence + ', ' + (gpt6InitialPower + 1) + ' of 5.</span>' +
              '<span>Use Left and Right arrow keys to adjust power.</span>' +
            '</div></div>' +
          '<div data-testid="composer-model-picker-slider-advanced-view"><div role="group">' +
            gpt6Versions.map((version) => (
              '<div role="menuitemradio" tabindex="0" data-version="' + version + '" aria-checked="' +
                String(version === gpt6InitialVersion) + '"><div>' + version + '</div></div>'
            )).join('') +
          '</div></div>' +
        '</div>' +
      '</div>' +
    '</div>';
  const legacyPopper =
    '<div data-radix-popper-content-wrapper id="intelligence-popper" hidden>' +
      '<div data-testid="composer-intelligence-picker-content" aria-label="Intelligence">' +
        '<div role="menu" aria-label="Intelligence">' + intelligenceRadios +
          '<button type="button" role="menuitem">GPT-5.6 Sol</button>' +
        '</div>' +
      '</div>' +
    '</div>';
  app.innerHTML =
    '<section id="conversation"></section>' +
    '<form id="composer-form" onsubmit="return false">' +
    '<button type="button" id="composer-plus" aria-haspopup="menu" aria-label="Add files and more"><svg aria-hidden="true"></svg></button>' +
    '<button type="button" class="__composer-pill" id="intelligence-pill" aria-haspopup="menu" ' +
      'aria-expanded="false" data-open-count="0">' + (gpt6 ? gpt6PillMarkup(initialIntelligence) : intelligenceMarkup(initialIntelligence)) +
      '<svg aria-hidden="true"></svg></button>' +
    '<div id="chips"></div><input type="file" multiple aria-label="Attach files">' +
    '<div id="prompt-textarea" contenteditable="true" role="textbox"></div>' +
    '<button type="button" data-testid="send-button" aria-label="Send prompt">Send</button></form>' +
    (gpt6 ? gpt6Popper : legacyPopper);
  const intelligencePill = document.getElementById('intelligence-pill');
  // 2026-09-05 실 UI: + 메뉴는 group/text, Create image는 편집기 안에 도구 표시를 넣는다.
  const plus = document.getElementById('composer-plus');
  plus.addEventListener('click', () => {
    plus.setAttribute('aria-expanded', 'true');
    const group = document.createElement('div');
    group.setAttribute('role', 'group');
    const option = document.createElement('span');
    option.textContent = 'Create image';
    option.addEventListener('click', () => {
      document.getElementById('prompt-textarea').replaceChildren('Create image ');
      plus.setAttribute('aria-expanded', 'false');
      group.remove();
    });
    group.append(option);
    app.append(group);
  });
  const intelligencePopper = document.getElementById('intelligence-popper');
  const intelligenceItems = Array.from(app.querySelectorAll('[role="menu"] [role="menuitemradio"]'));
  intelligencePill.addEventListener('click', () => {
    intelligencePill.dataset.openCount = String(Number(intelligencePill.dataset.openCount) + 1);
    intelligencePill.setAttribute('aria-expanded', 'true');
    setTimeout(() => { intelligencePopper.hidden = false; }, 100);
  });
  if (gpt6) {
    const slider = document.getElementById('power-slider');
    const status = document.getElementById('power-status');
    const headerPower = document.getElementById('model-select-power');
    const setPower = (index) => {
      const bounded = Math.max(0, Math.min(4, index));
      slider.setAttribute('aria-valuenow', String(bounded));
      status.textContent = POWER_LEVELS[bounded] + ', ' + (bounded + 1) + ' of 5.';
      headerPower.textContent = POWER_LEVELS[bounded];
      intelligencePill.innerHTML = gpt6PillMarkup(POWER_LEVELS[bounded]) + '<svg aria-hidden="true"></svg>';
    };
    const powerInput = powerMenu ? document.getElementById('power-control') : slider;
    powerInput.addEventListener('keydown', (event) => {
      if (scenario === 'gpt6-slider-stuck' || scenario === 'gpt6-power-menu-stuck') return;
      const now = Number(slider.getAttribute('aria-valuenow'));
      if (event.key === 'End' && !powerMenu) setPower(4);
      else if (event.key === 'Home' && !powerMenu) setPower(0);
      else if (event.key === 'ArrowRight') setPower(now + 1);
      else if (event.key === 'ArrowLeft') setPower(now - 1);
      else return;
      event.preventDefault();
    });
    const modelSelect = document.getElementById('model-select');
    modelSelect.addEventListener('click', () => {
      modelSelect.setAttribute('aria-expanded', modelSelect.getAttribute('aria-expanded') === 'true' ? 'false' : 'true');
    });
    for (const item of intelligenceItems) {
      item.addEventListener('click', () => {
        if (versionConfirmation) {
          intelligencePopper.hidden = true;
          intelligencePill.setAttribute('aria-expanded', 'false');
          if (scenario === 'gpt6-model-rejected-closes') return;
        }
        for (const candidate of intelligenceItems) candidate.setAttribute('aria-checked', 'false');
        item.setAttribute('aria-checked', 'true');
      });
    }
    if (scenario === 'gpt6-model-missing-controls') {
      document.getElementById('power-control').remove();
      modelSelect.remove();
    }
    document.addEventListener('keydown', (event) => {
      if (event.key !== 'Escape' || intelligencePopper.hidden) return;
      intelligencePopper.hidden = true;
      intelligencePill.setAttribute('aria-expanded', 'false');
    });
  } else {
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
    if (scenario === 'image-single') renderedPrompt = prompt.replace(/\s+/g, ' ');
    const conversation = document.getElementById('conversation');
    const user = document.createElement('div');
    user.setAttribute('data-message-author-role', 'user');
    user.setAttribute('data-message-id', 'user-' + tabId + '-' + (++sequence));
    const attachmentPrefix = scenario === 'attachments'
      ? Array.from(chips.querySelectorAll('span')).map((node) => node.textContent + '\nFile').join('\n')
      : '';
    user.textContent = attachmentPrefix ? attachmentPrefix + '\n' + renderedPrompt : renderedPrompt;
    if (scenario === 'image-single') {
      const toggle = document.createElement('button');
      toggle.dataset.testid = 'collapsible-user-message-toggle';
      toggle.textContent = 'Show moreShow less';
      user.append(toggle);
    }
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
    if (currentScenario === 'image-set' || currentScenario === 'image-single') {
      assistant.textContent = '';
      const gallery = document.createElement('div');
      let selected = 0;
      const preview = document.createElement('div');
      preview.setAttribute('role', 'button');
      const previewImage = document.createElement('img');
      previewImage.alt = currentScenario === 'image-single' ? 'Generated image: Seongsu Alley and Workshop Walk' : 'Generated image';
      preview.setAttribute('aria-label', previewImage.alt);
      previewImage.src = '/preview/image-0.png';
      previewImage.width = 256;
      previewImage.height = 192;
      preview.append(previewImage);
      preview.addEventListener('click', () => {
        const dialog = document.createElement('div');
        dialog.setAttribute('role', 'dialog');
        dialog.setAttribute('aria-label', currentScenario === 'image-single' ? 'Seongsu Alley and Workshop Walk' : 'Media viewer');
        const imageTools = document.createElement('div');
        imageTools.setAttribute('role', 'group'); imageTools.setAttribute('aria-label', 'Image tools');
        dialog.append(imageTools);
        const button = document.createElement('button');
        button.setAttribute('aria-label', 'Save');
        button.textContent = 'Save';
        button.addEventListener('click', () => {
          if (currentScenario === 'image-single') {
            const link = document.createElement('a');
            link.href = '/download/image-' + selected + '.png'; link.download = 'image.png';
            dialog.append(link); link.click(); link.remove();
            return;
          }
          const menu = document.createElement('div');
          menu.setAttribute('role', 'menu'); menu.setAttribute('aria-label', 'Save');
          const item = document.createElement('button');
          item.setAttribute('role', 'menuitem'); item.textContent = 'Download image';
          item.addEventListener('click', () => {
            const link = document.createElement('a');
            link.href = '/download/image-' + selected + '.png';
            link.download = 'image.png';
            dialog.append(link); link.click(); link.remove(); menu.remove();
          });
          const series = document.createElement('button'); series.setAttribute('role', 'menuitem');
          series.textContent = 'Download 5 images in this series';
          menu.append(item, series); dialog.append(menu);
        });
        dialog.append(button);
        app.append(dialog);
        const close = (event) => { if (event.key === 'Escape') { dialog.remove(); document.removeEventListener('keydown', close); } };
        document.addEventListener('keydown', close);
      });
      gallery.append(preview);
      for (let index = 0; index < (currentScenario === 'image-single' ? 0 : 5); index += 1) {
        const thumbnail = document.createElement('button');
        thumbnail.dataset.imageIndex = String(index);
        for (let layer = 0; layer < 3; layer += 1) {
          const image = document.createElement('img'); image.alt = 'Generated image';
          image.src = '/preview/image-' + index + '.png'; image.width = 24; image.height = 18;
          thumbnail.append(image);
        }
        thumbnail.addEventListener('click', () => {
          setTimeout(() => { selected = index; previewImage.src = '/preview/image-' + index + '.png'; }, 150);
        });
        gallery.append(thumbnail);
      }
      // 실 이미지 세트는 텍스트 메시지 노드 밖에 렌더된다.
      assistant.parentElement.append(gallery);
      assistant.parentElement.append(actionBar);
    }
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
