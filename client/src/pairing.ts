// Pairing links and certificate fingerprints, without editor dependencies (unit tested).

export interface PairingLink {
	/** Host base URL, e.g. https://devbox:8443 */
	url: string;
	code: string;
	/** SHA-256 of the host's certificate (lowercase hex), for self-signed hosts. */
	fingerprint?: string;
}

export function normalizeFingerprint(fp: string): string {
	return fp.replace(/[:\s]/g, '').toLowerCase();
}

export function isFingerprint(fp: string): boolean {
	return /^[0-9a-f]{64}$/.test(normalizeFingerprint(fp));
}

/**
 * Accepts the VSCodium link `vscodium://remotium.client/pair?host=…&code=…&fp=…` (any editor
 * scheme) or the browser link `https://host:port/pair?code=…` (no fingerprint: only works for
 * hosts with a publicly trusted certificate).
 */
export function parsePairingLink(text: string): PairingLink {
	const link = text.trim();
	let u: URL;
	try {
		u = new URL(link);
	} catch {
		throw new Error('That is not a pairing link.');
	}
	const code = u.searchParams.get('code') ?? '';
	if (!/^rmp_[A-Za-z0-9_-]{20,}$/.test(code)) {
		throw new Error('The pairing link has no valid code.');
	}
	let url: string;
	if (u.protocol === 'https:' || u.protocol === 'http:') {
		if (!u.pathname.replace(/\/+$/, '').endsWith('/pair')) {
			throw new Error('That is not a pairing link.');
		}
		url = `${u.protocol}//${u.host}${u.pathname.replace(/\/pair\/?$/, '')}`;
	} else {
		const host = u.searchParams.get('host');
		if (!host) {
			throw new Error('The pairing link has no host.');
		}
		const h = new URL(host);
		if (h.protocol !== 'https:' && h.protocol !== 'http:') {
			throw new Error('The pairing link has an invalid host.');
		}
		url = `${h.protocol}//${h.host}${h.pathname.replace(/\/+$/, '')}`;
	}
	const fp = u.searchParams.get('fp') ?? undefined;
	if (fp !== undefined && !isFingerprint(fp)) {
		throw new Error('The pairing link has an invalid certificate fingerprint.');
	}
	return { url: url.replace(/\/+$/, ''), code, fingerprint: fp ? normalizeFingerprint(fp) : undefined };
}

/** `https://h:8443` → `wss://h:8443` */
export function toWebSocketUrl(url: string): string {
	return url.replace(/^http/, 'ws');
}
