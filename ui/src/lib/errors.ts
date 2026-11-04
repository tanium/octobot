export class HttpError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

// The server rejected the session. The API client has already cleared it,
// which sends the user back to the login page, and said so in a banner.
export class SessionExpiredError extends HttpError {
  constructor() {
    super(403, 'Session expired');
  }
}
