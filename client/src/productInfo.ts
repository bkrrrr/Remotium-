// Product discovery adapted from open-remote-ssh (MIT, see THIRD_PARTY_NOTICES.md).
import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';

/** The editor build, which the remote server must match exactly. */
export interface ProductInfo {
	commit: string;
	quality?: string;
	version: string;
	release?: string;
	urlProtocol: string;
	nameShort: string;
	dataFolderName: string;
}

let cached: ProductInfo | undefined;

export function getProductInfo(): ProductInfo {
	if (cached) {
		return cached;
	}
	const productPath = path.join(vscode.env.appRoot, 'product.json');
	const product = JSON.parse(fs.readFileSync(productPath, 'utf8'));
	const pkg = JSON.parse(fs.readFileSync(path.join(vscode.env.appRoot, 'package.json'), 'utf8'));
	const commit: string | undefined = product.commit ?? vscode.env.appCommit;
	if (!commit) {
		throw new Error('This editor build has no commit in product.json, so no matching server can be found.');
	}
	cached = {
		commit,
		quality: product.quality ?? vscode.env.appQuality,
		version: product.version ?? pkg.version,
		release: product.release,
		urlProtocol: product.urlProtocol ?? vscode.env.uriScheme,
		nameShort: product.nameShort ?? vscode.env.appName,
		dataFolderName: product.dataFolderName ?? '.vscode-oss',
	};
	return cached;
}
