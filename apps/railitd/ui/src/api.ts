// Zugang zur API von railitd über den HTTP-Client von bQuery.

import { createHttp, signal } from '@bquery/bquery/reactive';
import type { HttpClient } from '@bquery/bquery/reactive';
import type { Answer, Layout, NamedAux, PendingAux, Snapshot } from './types';

const TOKEN_KEY = 'railit.token';
const OPERATOR_KEY = 'railit.operator';

function stored(key: string): string {
  try {
    return sessionStorage.getItem(key) ?? '';
  } catch {
    return '';
  }
}

function store(key: string, value: string): void {
  try {
    sessionStorage.setItem(key, value);
  } catch {
    // Ohne Speicher gilt der Wert nur bis zum Neuladen.
  }
}

/** Zugangstoken (leer: offener Betrieb oder mTLS). */
export const token = signal(stored(TOKEN_KEY));
/** Bedienernummer für Hilfshandlungen (ohne Benutzerverwaltung des Dienstes). */
export const operator = signal(stored(OPERATOR_KEY));

export function setToken(value: string): void {
  token.value = value.trim();
  store(TOKEN_KEY, token.value);
}

export function setOperator(value: string): void {
  operator.value = value.trim();
  store(OPERATOR_KEY, operator.value);
}

/** Anmeldekopf und Bedienernummer für jede Anfrage. */
export function authHeaders(): Record<string, string> {
  const headers: Record<string, string> = {};
  if (token.value) headers.Authorization = `Bearer ${token.value}`;
  if (/^\d+$/.test(operator.value)) headers['X-Railit-Operator'] = operator.value;
  return headers;
}

/** `fetch` mit Anmeldekopf (für `usePolling`). */
export const authFetch = (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
  const headers = new Headers(init?.headers);
  for (const [k, v] of Object.entries(authHeaders())) headers.set(k, v);
  return fetch(input, { ...init, headers, cache: 'no-store' });
};

/** Fehler der Verbindung oder der Anmeldung. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

function client(): HttpClient {
  const http = createHttp({
    timeout: 5000,
    // Ablehnungen des Kerns (409) und Bedienfehler (400) sind Antworten, keine Ausnahmen.
    validateStatus: (status) => status < 500 && status !== 401 && status !== 403,
  });
  http.interceptors.request.use((config) => ({
    ...config,
    headers: { ...(config.headers as Record<string, string> | undefined), ...authHeaders() },
  }));
  return http;
}

const http = client();

async function call<T>(run: () => Promise<{ data: T; status: number }>): Promise<{ data: T; status: number }> {
  try {
    return await run();
  } catch (e) {
    const status = (e as { response?: { status?: number } }).response?.status ?? 0;
    const message =
      status === 401
        ? 'nicht angemeldet'
        : status === 403
          ? 'keine Berechtigung'
          : `keine Verbindung zu railitd (${(e as Error).message})`;
    throw new ApiError(status, message);
  }
}

export async function getLayout(): Promise<Layout> {
  return (await call(() => http.get<Layout>('/v1/layout'))).data;
}

export async function getState(): Promise<Snapshot> {
  return (await call(() => http.get<Snapshot>('/v1/state'))).data;
}

/** Bedienanforderung; Ergebnis ist die Antwort des Kerns bzw. eine Fehlermeldung. */
export async function request(path: string, body?: Record<string, unknown>): Promise<Answer> {
  const r = await call(() => http.post<Answer & { error?: string }>(path, body ?? null));
  if (r.status >= 400 && r.status !== 409) {
    return { type: 'rejected', reason: r.data?.error ?? `HTTP ${r.status}` };
  }
  return r.data;
}

const seg = (name: string) => encodeURIComponent(name);

export const setRoute = (route: string) => request(`/v1/routes/${seg(route)}/set`);
export const cancelRoute = (route: string) => request(`/v1/routes/${seg(route)}/cancel`);
export const movePoint = (point: string, position: 'left' | 'right') =>
  request(`/v1/points/${seg(point)}/move`, { position });
export const requestPermission = (address: string) => request(`/v1/borders/${seg(address)}/request`);
export const requestAux = (op: NamedAux) => request('/v1/aux', { ...op });

/**
 * Bestätigung über die sichere Anzeige- und Bestätigungskette: `restated` ist die
 * Hilfshandlung, wie der zweite Bediener sie eingegeben hat, `check` der aus der Anzeige
 * berechnete Prüfcode.
 */
export const confirmAux = (pending: PendingAux, restated: NamedAux, check: string) =>
  request(`/v1/aux/${pending.aux}/confirm?area=${seg(pending.area)}`, { ...restated, check });

/** Ereignisstrom (Server-Sent Events) mit Anmeldekopf; verbindet sich selbst neu. */
export function streamEvents(onEvent: (event: Record<string, unknown>) => void): () => void {
  let stopped = false;
  let controller: AbortController | undefined;
  const run = async () => {
    while (!stopped) {
      controller = new AbortController();
      try {
        const response = await authFetch('/v1/events', { signal: controller.signal });
        const reader = response.body?.getReader();
        if (!reader) throw new Error('kein Ereignisstrom');
        const decoder = new TextDecoder();
        let buffer = '';
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          buffer += decoder.decode(value, { stream: true });
          let end: number;
          while ((end = buffer.indexOf('\n\n')) >= 0) {
            const block = buffer.slice(0, end);
            buffer = buffer.slice(end + 2);
            const data = block
              .split('\n')
              .filter((l) => l.startsWith('data:'))
              .map((l) => l.slice(5).trim())
              .join('');
            if (data) {
              try {
                onEvent(JSON.parse(data) as Record<string, unknown>);
              } catch {
                // Unlesbares Ereignis überspringen.
              }
            }
          }
        }
      } catch {
        // Neuer Versuch nach kurzer Pause.
      }
      if (!stopped) await new Promise((r) => setTimeout(r, 1000));
    }
  };
  void run();
  return () => {
    stopped = true;
    controller?.abort();
  };
}
