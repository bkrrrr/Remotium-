/** Minimal client for the server's control API. */

export class ServerError extends Error {
	constructor(readonly status: number, readonly code: string, message: string) {
		super(message);
	}
	/** The sign-in is gone for good; only a new launch from the dashboard helps. */
	get isAuthFailure(): boolean {
		return this.status === 401 || this.status === 403;
	}
}

export interface TokenPair {
	accessToken: string;
	accessExpiresIn: number;
	refreshToken: string;
	refreshExpiresIn: number;
	user: string;
	env: string;
	project?: string;
}

export interface SessionInfo {
	sessionId: string;
	streamUrl: string;
	streamToken: string;
	streamTokenExpiresIn: number;
	connectionToken: string;
}

export interface ServerVersion {
	commit: string;
	quality?: string;
	version: string;
	release?: string;
}

export class ServerApi {
	constructor(readonly baseUrl: string) {}

	private async call<T>(path: string, body: unknown, bearer?: string, timeoutMs = 20_000): Promise<T> {
		const headers: Record<string, string> = { 'content-type': 'application/json', accept: 'application/json' };
		if (bearer) {
			headers.authorization = `Bearer ${bearer}`;
		}
		let res: Response;
		try {
			res = await fetch(new URL(path, this.baseUrl.endsWith('/') ? this.baseUrl : `${this.baseUrl}/`), {
				method: 'POST',
				headers,
				body: JSON.stringify(body ?? {}),
				signal: AbortSignal.timeout(timeoutMs),
			});
		} catch (e) {
			const what = (e as Error).name === 'TimeoutError' ? `no answer within ${timeoutMs / 1000} s` : (e as Error).message;
			throw new ServerError(0, 'network', `cannot reach server ${this.baseUrl} (${path}): ${what}`);
		}
		if (res.status === 204) {
			return undefined as T;
		}
		const text = await res.text();
		let json: any;
		try {
			json = text ? JSON.parse(text) : {};
		} catch {
			json = { error: 'bad_response', message: text.slice(0, 200) };
		}
		if (!res.ok) {
			throw new ServerError(res.status, json.error ?? 'error', json.message ?? `HTTP ${res.status}`);
		}
		return json as T;
	}

	redeemTicket(ticket: string): Promise<TokenPair> {
		return this.call('api/v1/tokens', { ticket });
	}

	refresh(refreshToken: string): Promise<TokenPair> {
		return this.call('api/v1/tokens', { refreshToken });
	}

	revokeTokens(refreshToken: string): Promise<void> {
		return this.call('api/v1/tokens/revoke', { refreshToken });
	}

	createSession(accessToken: string, envId: string, v: ServerVersion): Promise<SessionInfo> {
		// May wait for the environment to download a matching server.
		return this.call('api/v1/sessions', { envId, ...v }, accessToken, 11 * 60_000);
	}

	streamToken(accessToken: string, sessionId: string): Promise<{ streamToken: string; expiresIn: number }> {
		return this.call(`api/v1/sessions/${encodeURIComponent(sessionId)}/stream-tokens`, {}, accessToken);
	}

	revokeSession(accessToken: string, sessionId: string): Promise<void> {
		return this.call(`api/v1/sessions/${encodeURIComponent(sessionId)}/revoke`, {}, accessToken);
	}
}
