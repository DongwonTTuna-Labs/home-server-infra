import { randomBytes } from "node:crypto";
export function newRequestId(): string {
  return `req_${randomBytes(8).toString("hex")}`;
}
export function isRequestId(value: string): boolean {
  return /^req_[0-9a-f]{16}$/.test(value);
}
