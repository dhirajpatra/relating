/** Typed errors preserving the server's `{error: {code, message}}` shape. */
export class RelatingApiError extends Error {
  readonly status: number;
  readonly code?: string;

  constructor(message: string, status: number, code?: string) {
    super(code ? `[${code} HTTP ${status}] ${message}` : message);
    this.name = 'RelatingApiError';
    this.status = status;
    this.code = code;
  }
}

export function errorFromResponse(status: number, payload: unknown): RelatingApiError {
  let code: string | undefined;
  let message = `HTTP ${status}`;
  if (typeof payload === 'object' && payload !== null) {
    const error = (payload as Record<string, unknown>).error;
    if (typeof error === 'object' && error !== null) {
      const record = error as Record<string, unknown>;
      if (typeof record.code === 'string') code = record.code;
      if (typeof record.message === 'string' && record.message) message = record.message;
    }
  }
  return new RelatingApiError(message, status, code);
}
