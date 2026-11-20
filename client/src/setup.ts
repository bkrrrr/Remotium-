import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import { addProposal, argvHasProposal, argvPath, EXTENSION_ID } from './argv';
import { getProductInfo } from './productInfo';

export interface SetupOptions {
	/** Ask first. `false` is for scripted setups and tests. */
	confirm?: boolean;
	/** A blocking dialog (used when the user just clicked a launch link). */
	modal?: boolean;
}

/**
 * VSCodium doesn't let the extension use its remote connection API yet. Offer to add it to
 * argv.json; the change applies after VSCodium is restarted. Returns true if the file is set up.
 */
export async function setUp(log: vscode.LogOutputChannel, opts: SetupOptions = {}): Promise<boolean> {
	const file = argvPath(getProductInfo().dataFolderName);
	let text = '';
	try {
		text = fs.readFileSync(file, 'utf8');
	} catch {
		// Missing: created below.
	}
	if (argvHasProposal(text)) {
		if (opts.confirm !== false) {
			void vscode.window.showWarningMessage('Remotium: setup is done. Quit VSCodium and start it again to finish.');
		}
		return true;
	}
	if (opts.confirm !== false) {
		const message = 'Remotium needs a one-time setup: VSCodium has to allow its remote connection API.';
		const detail = `This adds "${EXTENSION_ID}" to "enable-proposed-api" in ${file}. Afterwards, restart VSCodium.`;
		const choice = opts.modal
			? await vscode.window.showWarningMessage(message, { modal: true, detail }, 'Set Up')
			: await vscode.window.showWarningMessage(`${message} ${detail}`, 'Set Up');
		if (choice !== 'Set Up') {
			return false;
		}
	}
	try {
		fs.mkdirSync(path.dirname(file), { recursive: true });
		fs.writeFileSync(file, addProposal(text));
	} catch (e) {
		log.error(`updating ${file} failed: ${(e as Error).message}`);
		void vscode.window.showErrorMessage(`Remotium: couldn't update ${file}: ${(e as Error).message}`);
		return false;
	}
	log.info(`added ${EXTENSION_ID} to enable-proposed-api in ${file}`);
	if (opts.confirm !== false) {
		void vscode.window.showInformationMessage('Remotium is set up. Quit VSCodium and start it again, then open your environment from the dashboard.');
	}
	return true;
}
