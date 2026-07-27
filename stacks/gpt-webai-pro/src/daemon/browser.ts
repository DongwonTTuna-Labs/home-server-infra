import { chromium, type Browser, type Page } from "playwright-core";

import { GwpError, isDirectNetworkFailure } from "../shared/errors.js";

export class BrowserSession {
  readonly baseUrl: string;
  readonly baseOrigin: string;

  private constructor(
    private readonly browser: Browser,
    baseUrl: string,
  ) {
    this.baseUrl = baseUrl;
    this.baseOrigin = new URL(baseUrl).origin;
  }

  static async connect(cdpUrl: string, baseUrl: string): Promise<BrowserSession> {
    const browser = await chromium.connectOverCDP(cdpUrl);
    const session = new BrowserSession(browser, baseUrl);
    await session.ensureInitialPage();
    return session;
  }

  static async fromBrowser(browser: Browser, baseUrl: string): Promise<BrowserSession> {
    const session = new BrowserSession(browser, baseUrl);
    await session.ensureInitialPage();
    return session;
  }

  connected(): boolean {
    return this.browser.isConnected();
  }

  async inspectionPage(): Promise<Page | null> {
    return (await this.relevantPages()).at(-1) ?? null;
  }

  async newConversation(): Promise<Page> {
    const context = this.browser.contexts()[0] ?? await this.browser.newContext();
    const page = await context.newPage();
    await this.navigate(page, this.baseUrl);
    return page;
  }

  async open(conversationUrl: string): Promise<Page> {
    if (!this.isConversationUrl(conversationUrl)) {
      throw new GwpError("nav_failed", `rejected conversation URL: ${conversationUrl}`);
    }
    const existing = await this.findConversationPage(conversationUrl);
    if (existing) return existing;
    const context = this.browser.contexts()[0] ?? await this.browser.newContext();
    const page = await context.newPage();
    await this.navigate(page, conversationUrl);
    return page;
  }

  async findConversationPage(conversationUrl: string): Promise<Page | null> {
    return (await this.relevantPages()).find((page) => page.url() === conversationUrl) ?? null;
  }

  async closeConversation(conversationUrl: string): Promise<void> {
    for (const page of await this.relevantPages()) {
      if (page.url() === conversationUrl) await page.close().catch(() => undefined);
    }
  }

  async relevantPages(): Promise<Page[]> {
    const pages = this.browser.contexts().flatMap((context) => context.pages());
    const result: Page[] = [];
    for (const page of pages) {
      if (page.isClosed()) continue;
      try {
        if (new URL(page.url()).origin === this.baseOrigin) result.push(page);
      } catch {
        // Ignore browser-internal and transient URLs.
      }
    }
    return result;
  }

  isConversationUrl(value: string): boolean {
    try {
      const url = new URL(value);
      return url.origin === this.baseOrigin && /^\/c\/[^/?#]+\/?$/.test(url.pathname);
    } catch {
      return false;
    }
  }

  private async ensureInitialPage(): Promise<void> {
    if ((await this.relevantPages()).length > 0) return;
    const context = this.browser.contexts()[0] ?? await this.browser.newContext();
    const page = await context.newPage();
    await this.navigate(page, this.baseUrl);
  }

  private async navigate(page: Page, url: string): Promise<void> {
    try {
      await page.goto(url, { waitUntil: "domcontentloaded", timeout: 30_000 });
    } catch (error) {
      throw new GwpError("nav_failed", `navigation failed: ${String(error)}`, {
        cause: error,
        networkEvidence: isDirectNetworkFailure(error),
      });
    }
  }
}
