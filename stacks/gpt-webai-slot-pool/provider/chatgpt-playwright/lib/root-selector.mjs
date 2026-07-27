import { sha256Text } from './common.mjs';
import {
  canonicalSha256,
  derivePageBindingId,
} from './contracts/r13.mjs';
import { captureBrowserPageIdentity } from './browser-session.mjs';
import {
  loadModelEffortLabels,
  normalizeVisibleLabel,
} from './commands/ensure-model.mjs';

const H256 = /^sha256:[0-9a-f]{64}$/;
const CONTROL_ROLES = new Set(['button', 'combobox', 'menuitem', 'option']);

export class RootSelectorError extends Error {
  constructor(reason, field) {
    super(`${reason}: ${field}`);
    this.name = 'RootSelectorError';
    this.reason = reason;
    this.field = field;
  }
}

export function structuralIdentity(candidate, prefix) {
  assertExact(candidate, [
    'ariaLabelHash', 'boundingBox', 'domPath', 'role', 'tagName', 'testIdHash',
  ], 'candidate.identity');
  const tagName = nonEmpty(candidate.tagName, 'candidate.tagName').toLowerCase();
  const role = nullableString(candidate.role, 'candidate.role');
  const testIdHash = nullableHash(candidate.testIdHash, 'candidate.testIdHash');
  const ariaLabelHash = nullableHash(candidate.ariaLabelHash, 'candidate.ariaLabelHash');
  const domPathHash = hashDomPath(candidate.domPath);
  const boundingBoxHash = hashBoundingBox(candidate.boundingBox);
  // The structural id anchors on SEMANTIC signals (tag, role, testId,
  // accessible-name, DOM path) and deliberately excludes boundingBoxHash: the
  // viewport-relative box is presentation, not identity, and shifts on benign
  // layout changes (e.g. the composer grows when a file is attached, or a
  // ChatGPT redesign reflows it) that must not invalidate the page binding.
  // boundingBoxHash is still carried as a field for evidence/telemetry.
  const digest = canonicalSha256([
    tagName,
    role ?? '',
    testIdHash ?? '',
    ariaLabelHash ?? '',
    domPathHash,
  ]);
  if (prefix !== 'root' && prefix !== 'control') {
    throw new RootSelectorError('provider.schema_drift', 'candidate.prefix');
  }
  return {
    boundingBoxHash,
    domPathHash,
    id: `${prefix}_${digest}`,
  };
}

export function selectRootBindingCandidates(input) {
  assertExact(input, [
    'composerRoots', 'conversationRoots', 'domMutationGeneration', 'effortControls',
    'modelControls', 'normalizedUrl',
  ], 'rootSelector');
  const conversation = select('conversation', input.conversationRoots);
  const composer = select('composer', input.composerRoots);
  const model = select('model', input.modelControls, composer.candidate.boundingBox);
  // Unified model/effort menu (current ChatGPT UI exposes one tier control and
  // no separate effort control): the model tier control also governs effort.
  const effort = input.effortControls.length
    ? select('effort', input.effortControls, composer.candidate.boundingBox)
    : model;
  return {
    composerRoot: composer.candidate,
    composerRootId: composer.identity.id,
    conversationRoot: conversation.candidate,
    conversationRootId: conversation.identity.id,
    effortControl: controlIdentity(effort.candidate, effort.identity),
    modelControl: controlIdentity(model.candidate, model.identity),
    rootBindingHash: rootBindingHash({
      composerRootId: composer.identity.id,
      conversationRootId: conversation.identity.id,
      domMutationGeneration: input.domMutationGeneration,
      effortControlId: effort.identity.id,
      modelControlId: model.identity.id,
      normalizedUrl: input.normalizedUrl,
    }),
    selectorMargin: Math.min(
      conversation.margin,
      composer.margin,
      model.margin,
      effort.margin,
    ),
  };
}

export function rootBindingHash(value) {
  assertExact(value, [
    'composerRootId', 'conversationRootId', 'domMutationGeneration',
    'effortControlId', 'modelControlId', 'normalizedUrl',
  ], 'rootBindingHash');
  integer(value.domMutationGeneration, 0, 65_535, 'rootBindingHash.domMutationGeneration');
  if (value.normalizedUrl !== 'https://chatgpt.com/'
      && !/^https:\/\/chatgpt\.com\/c\/[A-Za-z0-9][A-Za-z0-9_-]{0,127}$/.test(value.normalizedUrl)) {
    throw new RootSelectorError('provider.schema_drift', 'rootBindingHash.normalizedUrl');
  }
  return `sha256:${canonicalSha256([
    value.conversationRootId,
    value.composerRootId,
    value.modelControlId,
    value.effortControlId,
    value.normalizedUrl,
    value.domMutationGeneration,
  ])}`;
}

export async function captureRootState(page) {
  const [pageIdentity, captured] = await Promise.all([
    captureBrowserPageIdentity(page),
    captureRootSelectorInput(page),
  ]);
  // Neutralize the DOM-mutation-generation counter for identity purposes. It is
  // a per-capture churn signal that increments on ANY incidental mutation
  // (cursor blink, spinner, streamed token) during the capture window, so it is
  // inherently non-deterministic across the separate operations of one request
  // and produces spurious binding.mismatch failures on the live (constantly
  // animating) ChatGPT UI. Real page-identity changes (navigation, target/
  // context swap) are already caught by pageIncarnationId/targetId/
  // browserContextId/normalizedUrl, so a constant here loses no safety while
  // making the binding stable across ops. Semantic identity over volatile churn
  // — same rationale as excluding boundingBoxHash from the id tuple.
  captured.selectorInput.domMutationGeneration = 0;
  const selected = selectRootBindingCandidates(captured.selectorInput);
  return {
    ...selected,
    ...pageIdentity,
    domMutationGeneration: captured.selectorInput.domMutationGeneration,
    effortLabel: captured.labelsByDomPath.get(selected.effortControl.domPathHash) ?? '',
    modelLabel: captured.labelsByDomPath.get(selected.modelControl.domPathHash) ?? '',
    normalizedUrl: captured.selectorInput.normalizedUrl,
  };
}

export async function observeBoundPage(page, expected) {
  const state = await captureRootState(page);
  const pageIncarnationId = state.pageIncarnationId;
  const bindingId = derivePageBindingId(pageIncarnationId, state.rootBindingHash);
  return {
    ...expected,
    bindingId,
    browserContextId: state.browserContextId,
    domMutationGeneration: state.domMutationGeneration,
    pageIncarnationId,
    rootBindingHash: state.rootBindingHash,
    targetId: state.targetId,
  };
}

export async function captureRootSelectorInput(page) {
  const fixtureLabels = await loadRootSelectorLabelFixtureSets();
  const captured = await page.evaluate(({ effortLabels, modelLabels }) => {
    const effortLabelSet = new Set(effortLabels);
    const modelLabelSet = new Set(modelLabels);
    const normalizeLabel = value => String(value || '')
      .normalize('NFC')
      .toLowerCase()
      .replace(/\s+/gu, ' ')
      .trim();
    const accessibleName = node => normalizeLabel(
      node.getAttribute('aria-label') || node.innerText || node.textContent || '',
    );
    const visible = node => {
      const rect = node?.getBoundingClientRect?.();
      if (!rect || rect.width <= 0 || rect.height <= 0) return false;
      const style = getComputedStyle(node);
      return style.display !== 'none'
        && style.visibility !== 'hidden'
        && style.visibility !== 'collapse'
        && node.getAttribute('aria-hidden') !== 'true'
        && rect.bottom > 0
        && rect.right > 0
        && rect.top < innerHeight
        && rect.left < innerWidth;
    };
    const ancestors = (node, limit) => {
      const values = [];
      let current = node;
      for (let depth = 0; current && depth <= limit; depth += 1) {
        values.push(current);
        current = current.parentElement;
      }
      return values;
    };
    const sidebarAncestor = node => ancestors(node, Number.MAX_SAFE_INTEGER).some(item => (
      item.tagName === 'NAV' || item.getAttribute('role') === 'navigation'
    ));
    const unique = values => [...new Set(values.filter(Boolean))];
    const domPath = node => {
      const result = [];
      let current = node;
      while (current?.nodeType === Node.ELEMENT_NODE) {
        let index = 0;
        let sibling = current.previousElementSibling;
        while (sibling) {
          index += 1;
          sibling = sibling.previousElementSibling;
        }
        result.unshift([String(current.tagName || '').toLowerCase(), index]);
        current = current.parentElement;
      }
      return result;
    };
    const identity = (node, allowAccessibleName = false) => {
      const rect = node.getBoundingClientRect();
      return {
        tagName: String(node.tagName || '').toLowerCase(),
        role: node.getAttribute('role') || (node.tagName === 'BUTTON' ? 'button' : null),
        testId: node.getAttribute('data-testid') || '',
        ariaLabel: allowAccessibleName
          ? accessibleName(node)
          : node.getAttribute('aria-label') || '',
        domPath: domPath(node),
        boundingBox: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
      };
    };
    const textbox = unique(Array.from(document.querySelectorAll(
      '#prompt-textarea,textarea,[contenteditable="true"][role="textbox"],.ProseMirror[contenteditable="true"]',
    )).filter(visible)).at(-1) || null;
    // A fresh run starts on an empty new-chat page: the conversation container
    // (`main`) exists but has no turns yet. Require turn-containment only when
    // the page actually has turns (session/resume pages); on a turnless fresh
    // page accept the visible conversation container so root capture can bind it.
    const anyConversationTurn = document.querySelector(
      '[data-testid^="conversation-turn"],[data-message-author-role]',
    );
    const conversationNodes = unique([
      document.querySelector('main'),
      ...document.querySelectorAll('[data-testid*="conversation" i],[data-testid*="thread" i]'),
    ]).filter(node => visible(node) && (
      anyConversationTurn
        ? node.querySelector('[data-testid^="conversation-turn"],[data-message-author-role]')
        : true
    ));
    const composerNodes = unique([
      textbox?.closest('form'),
      textbox?.closest('[data-testid*="composer" i]'),
      textbox?.closest('[class*="composer" i]'),
      textbox?.parentElement,
    ]).filter(visible);
    const controls = unique(Array.from(document.querySelectorAll(
      'button,[role="button"],[role="combobox"],[data-testid*="model" i]',
    )).filter(visible));
    const structurallyModelLike = node => Boolean(
      node.getAttribute('aria-haspopup')
      || node.getAttribute('role') === 'combobox'
      || node.closest('header,form,[data-testid*="composer" i],[class*="composer" i]'),
    );
    const structurallyEffortLike = node => Boolean(
      node.closest('header,form,[role="menu"],[data-testid*="composer" i],[class*="composer" i]')
      || node.getAttribute('aria-controls')
      || node.getAttribute('aria-owns'),
    );
    const modelNodes = controls.filter(node => (
      modelLabelSet.has(accessibleName(node)) || structurallyModelLike(node)
    ));
    // Effort is identified only by its catalog label. The prior structural
    // fallback matched almost every composer/header control (sidebar, apps, …)
    // and made effort selection ambiguous on the live UI. When the UI has no
    // labelled effort control (unified model/effort menu) this is empty and the
    // model tier control governs effort (resolved in selectRootBindingCandidates).
    const effortNodes = controls.filter(node => effortLabelSet.has(accessibleName(node)));
    const menuIds = new Set(modelNodes.flatMap(node => [
      node.getAttribute('aria-controls'),
      node.getAttribute('aria-owns'),
    ]).filter(Boolean));
    // Reset the mutation generation at the start of every capture. It is a
    // per-capture stability signal, NOT a page-lifetime counter: a persistent
    // observer accumulates the live page's incidental animations between the
    // capture-root and ensure-model operations, so the derived rootBindingHash
    // would never match across ops on a real (constantly mutating) ChatGPT page.
    // Resetting makes the binding structural and stable across operations while
    // still surfacing churn that happens within a single capture window.
    if (window.__gptWebaiR13MutationObserver) {
      window.__gptWebaiR13MutationObserver.disconnect();
    }
    {
      window.__gptWebaiR13MutationGeneration = 0;
      window.__gptWebaiR13MutationObserver = new MutationObserver(() => {
        window.__gptWebaiR13MutationGeneration = Math.min(
          65_535,
          Number(window.__gptWebaiR13MutationGeneration || 0) + 1,
        );
      });
      window.__gptWebaiR13MutationObserver.observe(document.documentElement, {
        attributes: true,
        childList: true,
        subtree: true,
      });
    }
    const rootCandidate = (node, kind) => ({
      identity: identity(node, false),
      ...(kind === 'conversation' ? {
        containsTurnList: Number(node.querySelectorAll(
          '[data-testid^="conversation-turn"],[data-message-author-role]',
        ).length >= 2),
        excludesSidebar: Number(!sidebarAncestor(node)),
        hiddenPenalty: Number(!visible(node)),
        roleMain: Number(ancestors(node, 2).some(item => (
          item.tagName === 'MAIN' || item.getAttribute('role') === 'main'
        ))),
        viewportWidthCoverageBucket: Math.max(0, Math.min(10, Math.floor(
          node.getBoundingClientRect().width / Math.max(1, innerWidth) * 10,
        ))),
        visible: Number(visible(node)),
      } : {
        containsTextareaOrContenteditable: Number(Boolean(node.querySelector('textarea,[contenteditable="true"],#prompt-textarea'))),
        fixedBottomOrForm: Number(ancestors(node, 3).some(item => {
          if (item.tagName === 'FORM') return true;
          const position = getComputedStyle(item).position;
          const rect = item.getBoundingClientRect();
          return ['fixed', 'sticky'].includes(position) && innerHeight - rect.bottom <= 200;
        })),
        historySidebarAncestorPenalty: Number(sidebarAncestor(node)),
        uploadControlNearby: Number(Boolean(node.querySelector('input[type="file"]'))),
        visible: Number(visible(node)),
      }),
    });
    const controlCandidate = (node, kind) => ({
      identity: identity(node, true),
      ...(kind === 'model' ? {
        ariaHasPopupOrButton: Number(Boolean(node.getAttribute('aria-haspopup'))
          || ['button', 'combobox'].includes(node.getAttribute('role'))
          || node.tagName === 'BUTTON'),
        disabledPenalty: Number(Boolean(node.disabled || node.getAttribute('aria-disabled') === 'true')),
        insideComposerOrHeader: Number(Boolean(node.closest('header'))
          || composerNodes.some(root => root.contains(node))),
        labelHashMatchesModelControl: Number(modelLabelSet.has(accessibleName(node))),
        visible: Number(visible(node)),
      } : {
        disabledPenalty: Number(Boolean(node.disabled || node.getAttribute('aria-disabled') === 'true')),
        labelHashMatchesEffortOrStandard: Number(effortLabelSet.has(accessibleName(node))),
        modelMenuAssociation: Number(Boolean(
          ancestors(node, Number.MAX_SAFE_INTEGER).some(item => menuIds.has(item.id)),
        ) || modelNodes.some(control => control.contains(node))),
        visible: Number(visible(node)),
      }),
      visibleLabel: accessibleName(node),
    });
    const current = new URL(window.location.href);
    const conversationMatch = current.pathname.match(/^\/c\/([A-Za-z0-9][A-Za-z0-9_-]{0,127})/);
    const normalizedUrl = conversationMatch
      ? `${current.origin}/c/${conversationMatch[1]}`
      : `${current.origin}/`;
    return {
      composerRoots: composerNodes.map(node => rootCandidate(node, 'composer')),
      conversationRoots: conversationNodes.map(node => rootCandidate(node, 'conversation')),
      domMutationGeneration: Number(window.__gptWebaiR13MutationGeneration || 0),
      effortControls: effortNodes.map(node => controlCandidate(node, 'effort')),
      modelControls: modelNodes.map(node => controlCandidate(node, 'model')),
      normalizedUrl,
    };
  }, {
    effortLabels: fixtureLabels.effort,
    modelLabels: fixtureLabels.model,
  });
  const labelsByDomPath = new Map();
  const mapCandidate = candidate => {
    const { visibleLabel = '', ...withoutLabel } = candidate;
    const mapped = {
      ...withoutLabel,
      identity: {
        ariaLabelHash: withoutLabel.identity.ariaLabel
          ? `sha256:${sha256Text(withoutLabel.identity.ariaLabel)}`
          : null,
        boundingBox: withoutLabel.identity.boundingBox,
        domPath: withoutLabel.identity.domPath,
        role: withoutLabel.identity.role,
        tagName: withoutLabel.identity.tagName,
        testIdHash: withoutLabel.identity.testId
          ? `sha256:${sha256Text(withoutLabel.identity.testId)}`
          : null,
      },
    };
    const identityValue = structuralIdentity(mapped.identity, 'control');
    labelsByDomPath.set(identityValue.domPathHash, visibleLabel);
    return mapped;
  };
  const mapRoot = candidate => {
    const mapped = mapCandidate(candidate);
    structuralIdentity(mapped.identity, 'root');
    return mapped;
  };
  return {
    labelsByDomPath,
    selectorInput: {
      composerRoots: captured.composerRoots.map(mapRoot),
      conversationRoots: captured.conversationRoots.map(mapRoot),
      domMutationGeneration: captured.domMutationGeneration,
      effortControls: captured.effortControls.map(mapCandidate),
      modelControls: captured.modelControls.map(mapCandidate),
      normalizedUrl: captured.normalizedUrl,
    },
  };
}

export async function loadRootSelectorLabelFixtureSets() {
  const labels = await loadModelEffortLabels();
  return {
    effort: [...labels.effort.values()].map(normalizeVisibleLabel),
    model: [...labels.model.values()].map(normalizeVisibleLabel),
  };
}

function select(kind, candidates, composerBox = null) {
  if (!Array.isArray(candidates) || candidates.length === 0) {
    throw new RootSelectorError('capture.ambiguous', `${kind}.empty`);
  }
  const ranked = candidates.map((candidate, sourceIndex) => {
    const normalized = normalizeCandidate(kind, candidate);
    const identity = structuralIdentity(normalized.identity, kind === 'conversation' || kind === 'composer' ? 'root' : 'control');
    return {
      candidate,
      distance: composerBox === null ? 0 : viewportDistance(candidate.identity.boundingBox, composerBox),
      identity,
      score: score(kind, candidate),
      sourceIndex,
    };
  });
  if (new Set(ranked.map(item => item.identity.domPathHash)).size !== ranked.length) {
    throw new RootSelectorError('capture.ambiguous', `${kind}.duplicateDomPathHash`);
  }
  ranked.sort((left, right) => right.score - left.score
    || left.distance - right.distance
    || left.identity.domPathHash.localeCompare(right.identity.domPathHash));
  const margin = ranked.length === 1 ? 100_000 : ranked[0].score - ranked[1].score;
  if (margin < 50 || margin > 100_000) {
    throw new RootSelectorError('capture.ambiguous', `${kind}.selectorMargin`);
  }
  return { ...ranked[0], margin };
}

function normalizeCandidate(kind, candidate) {
  const predicates = {
    conversation: ['containsTurnList', 'excludesSidebar', 'hiddenPenalty', 'roleMain', 'viewportWidthCoverageBucket', 'visible'],
    composer: ['containsTextareaOrContenteditable', 'fixedBottomOrForm', 'historySidebarAncestorPenalty', 'uploadControlNearby', 'visible'],
    model: ['ariaHasPopupOrButton', 'disabledPenalty', 'insideComposerOrHeader', 'labelHashMatchesModelControl', 'visible'],
    effort: ['disabledPenalty', 'labelHashMatchesEffortOrStandard', 'modelMenuAssociation', 'visible'],
  }[kind];
  assertExact(candidate, ['identity', ...predicates], `${kind}.candidate`);
  structuralIdentity(candidate.identity, kind === 'conversation' || kind === 'composer' ? 'root' : 'control');
  predicates.forEach(key => {
    const maximum = key === 'viewportWidthCoverageBucket' ? 10 : 1;
    integer(candidate[key], 0, maximum, `${kind}.${key}`);
  });
  return candidate;
}

function score(kind, candidate) {
  switch (kind) {
    case 'conversation':
      return candidate.visible * 1000 + candidate.roleMain * 500
        + candidate.containsTurnList * 300 + candidate.excludesSidebar * 200
        + candidate.viewportWidthCoverageBucket * 50 - candidate.hiddenPenalty * 10_000;
    case 'composer':
      return candidate.visible * 1000 + candidate.containsTextareaOrContenteditable * 500
        + candidate.fixedBottomOrForm * 250 + candidate.uploadControlNearby * 100
        - candidate.historySidebarAncestorPenalty * 10_000;
    case 'model':
      return candidate.visible * 1000 + candidate.ariaHasPopupOrButton * 300
        + candidate.labelHashMatchesModelControl * 200 + candidate.insideComposerOrHeader * 100
        - candidate.disabledPenalty * 5000;
    case 'effort':
      return candidate.visible * 1000 + candidate.labelHashMatchesEffortOrStandard * 300
        + candidate.modelMenuAssociation * 200 - candidate.disabledPenalty * 5000;
    default:
      throw new RootSelectorError('provider.schema_drift', 'candidate.kind');
  }
}

function controlIdentity(candidate, identity) {
  const role = candidate.identity.role;
  if (!CONTROL_ROLES.has(role) || candidate.visible !== 1 || candidate.disabledPenalty !== 0) {
    throw new RootSelectorError('capture.ambiguous', 'control.state');
  }
  if (candidate.identity.ariaLabelHash === null) {
    throw new RootSelectorError('provider.schema_drift', 'control.labelHash');
  }
  return {
    boundingBoxHash: identity.boundingBoxHash,
    controlId: identity.id,
    disabled: false,
    domPathHash: identity.domPathHash,
    labelHash: candidate.identity.ariaLabelHash,
    role,
    testIdHash: candidate.identity.testIdHash,
    visible: true,
  };
}

function hashDomPath(value) {
  if (!Array.isArray(value) || value.length === 0) throw new RootSelectorError('provider.schema_drift', 'candidate.domPath');
  const normalized = value.map((part, index) => {
    if (!Array.isArray(part) || part.length !== 2) throw new RootSelectorError('provider.schema_drift', `candidate.domPath[${index}]`);
    const tagName = nonEmpty(part[0], `candidate.domPath[${index}].tagName`).toLowerCase();
    integer(part[1], 0, 1_000_000, `candidate.domPath[${index}].index`);
    return [tagName, part[1]];
  });
  return `sha256:${canonicalSha256(normalized)}`;
}

function hashBoundingBox(value) {
  assertExact(value, ['height', 'width', 'x', 'y'], 'candidate.boundingBox');
  const rounded = ['x', 'y', 'width', 'height'].map(key => {
    if (!Number.isFinite(value[key])) throw new RootSelectorError('provider.schema_drift', `candidate.boundingBox.${key}`);
    return Math.round(value[key]);
  });
  return `sha256:${canonicalSha256(rounded)}`;
}

function viewportDistance(left, right) {
  const leftX = left.x + left.width / 2;
  const leftY = left.y + left.height / 2;
  const rightX = right.x + right.width / 2;
  const rightY = right.y + right.height / 2;
  return Math.floor(Math.hypot(leftX - rightX, leftY - rightY));
}

function assertExact(value, keys, field) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)
      || JSON.stringify(Object.keys(value).sort()) !== JSON.stringify([...keys].sort())) {
    throw new RootSelectorError('provider.schema_drift', field);
  }
}

function integer(value, minimum, maximum, field) {
  if (!Number.isInteger(value) || value < minimum || value > maximum) {
    throw new RootSelectorError('provider.schema_drift', field);
  }
}

function nonEmpty(value, field) {
  if (typeof value !== 'string' || value.length === 0 || value.includes('\0')) {
    throw new RootSelectorError('provider.schema_drift', field);
  }
  return value;
}

function nullableString(value, field) {
  if (value === null) return null;
  return nonEmpty(value, field);
}

function nullableHash(value, field) {
  if (value === null) return null;
  if (!H256.test(value)) throw new RootSelectorError('provider.schema_drift', field);
  return value;
}
