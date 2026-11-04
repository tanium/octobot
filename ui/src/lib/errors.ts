export class HttpError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

// The server rejected the session. The API client has already cleared it and
// shown a banner, so the app is back on the login page.
export class SessionExpiredError extends HttpError {
  constructor() {
    super(403, 'Session expired');
  }
}
