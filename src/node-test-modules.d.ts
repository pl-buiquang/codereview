// Minimal typings for the few node modules a test uses (@types/node is not a dependency).
declare module "node:child_process" {
  export function execFileSync(file: string, args: string[], options?: { cwd?: string; env?: Record<string, string | undefined>; stdio?: string }): unknown;
}
declare module "node:fs" {
  export function chmodSync(path: string, mode: number): void;
  export function mkdirSync(path: string): void;
  export function mkdtempSync(prefix: string): string;
  export function readdirSync(path: string): string[];
  export function readFileSync(path: string, encoding: string): string;
  export function rmSync(path: string, options?: { recursive?: boolean; force?: boolean }): void;
  export function writeFileSync(path: string, data: string): void;
}
declare module "node:os" {
  export function tmpdir(): string;
}
declare module "node:path" {
  export function join(...parts: string[]): string;
}
