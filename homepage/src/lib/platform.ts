/*
  Progressive enhancement for the download blocks. The server renders both buttons, equal, each
  linking to its platform's file. With a desktop OS detected, this leads with that OS's button
  (data-os on the block), opens that OS's install steps, and, for a machine 1.0 does not run on
  (an Intel Mac, Linux, Windows on ARM), shows the note that says which version fits.
*/

export type OS = 'mac' | 'windows' | 'linux';

type UAData = {
  platform?: string;
  mobile?: boolean;
  getHighEntropyValues?: (hints: string[]) => Promise<{ architecture?: string; bitness?: string }>;
};

export function detectOS(): OS | null {
  if (typeof navigator === 'undefined') return null;
  const uaData = (navigator as Navigator & { userAgentData?: UAData }).userAgentData;
  if (uaData?.mobile) return null; // a phone cannot install a desktop app
  const hint = `${uaData?.platform ?? ''} ${navigator.userAgent}`.toLowerCase();
  if (/android|iphone|ipad|ipod|cros/.test(hint)) return null;
  if (hint.includes('mac')) return navigator.maxTouchPoints > 1 ? null : 'mac'; // iPadOS reports "Macintosh"
  if (hint.includes('win')) return 'windows';
  if (hint.includes('linux') || hint.includes('x11')) return 'linux';
  return null;
}

// Chromium reports the CPU: "x86" on an Intel Mac, "arm" on Windows on ARM. Safari and Firefox
// expose none, so anything but a clear answer keeps the default (Apple silicon, x64).
async function architecture(): Promise<string | undefined> {
  const uaData = (navigator as Navigator & { userAgentData?: UAData }).userAgentData;
  if (!uaData?.getHighEntropyValues) return undefined;
  try {
    return (await uaData.getHighEntropyValues(['architecture'])).architecture;
  } catch {
    return undefined;
  }
}

function showNote(root: ParentNode, note: string) {
  root.querySelectorAll<HTMLElement>(`[data-note="${note}"]`).forEach((el) => (el.hidden = false));
}

export function enhanceDownloads(root: ParentNode = document) {
  const os = detectOS();
  if (!os) return;
  if (os === 'linux') {
    showNote(root, 'linux');
    return;
  }
  root.querySelectorAll<HTMLElement>('[data-downloads]').forEach((el) => (el.dataset.os = os));
  root.querySelectorAll<HTMLDetailsElement>(`details[data-platform="${os}"]`).forEach((d) => (d.open = true));
  void architecture().then((arch) => {
    if (os === 'mac' && arch === 'x86') showNote(root, 'intel-mac');
    if (os === 'windows' && arch === 'arm') showNote(root, 'windows-arm');
  });
}
