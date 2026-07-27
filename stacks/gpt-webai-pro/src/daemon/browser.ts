import { chromium, type Browser, type Page } from "playwright-core";

import { GwpError, isDirectNetworkFailure } from "../shared/errors.js";

export class BrowserSession {
  private page: Page | null = null;
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
    await session.currentPage();
    return session;
  }

  static async fromBrowser(browser: Browser, baseUrl: string): Promise<BrowserSession> {
    const session = new BrowserSession(browser, baseUrl);
    await session.currentPage();
    return session;
  }

  connected(): boolean {
    return this.browser.isConnected();
  }

  async currentPage(): Promise<Page> {
    if (this.page && !this.page.isClosed()) return this.page;
    const relevant = await this.relevantPages();
    this.page = relevant.at(-1) ?? null;
    if (this.page) return this.page;
    const context = this.browser.contexts()[0] ?? await this.browser.newContext();
    this.page = await context.newPage();
    await this.navigate(this.page, this.baseUrl);
    return this.page;
  }

  bindPage(page: Page): Page {
    if (page.isClosed()) throw new GwpError("nav_failed", "cannot bind a closed browser page");
    this.page = page;
    return page;
  }

  async newConversation(): Promise<Page> {
    const page = await this.currentPage();
    await this.navigate(page, this.baseUrl);
    this.page = page;
    return page;
  }

  async open(conversationUrl: string): Promise<Page> {
    if (!this.isConversationUrl(conversationUrl)) {
      throw new GwpError("nav_failed", `rejected conversation URL: ${conversationUrl}`);
    }
    const page = await this.currentPage();
    if (page.url() !== conversationUrl) await this.navigate(page, conversationUrl);
    this.page = page;
    return page;
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
