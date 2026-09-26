/*
  Progressive enhancement for the primary download button. The server-rendered <a> points at the
  latest release page and reads "Download Inkwell", which is right for everyone without JS, for
  phones and for anything undetected. With a desktop OS detected, this points it at that OS's file
  (the choices are written into data-choices by the server, from the release's real asset list),
  says which file it is, and marks that OS in the platform lists.
*/

export type OS = 'macOS' | 'Windows' | 'Linux';
type Choice = { href: string; note: string };

type UAData = {
  platform?: string;
  mobile?: boolean;
  getHighEntropyValues?: (hints: string[]) => Promise<{ architecture?: string }>;
};

export function detectOS(): OS | null {
  if (typeof navigator === 'undefined') return null;
  const uaData = (navigator as Navigator & { userAgentData?: UAData }).userAgentData;
  if (uaData?.mobile) return null; // a phone cannot install a desktop app: keep the neutral link
  const hint = `${uaData?.platform ?? ''} ${navigator.userAgent}`.toLowerCase();
  if (/android|iphone|ipad|ipod|cros/.test(hint)) return null;
  if (hint.includes('mac')) return navigator.maxTouchPoints > 1 ? null : 'macOS'; // iPadOS reports "Macintosh"
  if (hint.includes('win')) return 'Windows';
  if (hint.includes('linux') || hint.includes('x11')) return 'Linux';
  return null;
}

const isChoice = (c: unknown): c is Choice =>
  !!c && typeof (c as Choice).href === 'string' && /^https:\/\/github\.com\//.test((c as Choice).href) && typeof (c as Choice).note === 'string';

// Chromium on an Intel Mac reports "x86". Safari and Firefox expose no architecture, and every Mac
// browser says "Intel Mac OS X" in its user agent, so anything but a clear "x86" keeps Apple Silicon.
async function isIntelMac(): Promise<boolean> {
  const uaData = (navigator as Navigator & { userAgentData?: UAData }).userAgentData;
  if (!uaData?.getHighEntropyValues) return false;
  try {
    return (await uaData.getHighEntropyValues(['architecture'])).architecture === 'x86';
  } catch {
    return false;
  }
}

function applyChoice(root: ParentNode, key: string, os: OS) {
  root.querySelectorAll<HTMLAnchorElement>('a[data-choices]').forEach((a) => {
    let choice: unknown;
    try {
      choice = (JSON.parse(a.dataset.choices ?? '{}') as Record<string, unknown>)[key];
    } catch {
      return; // the release page link stays
    }
    if (!isChoice(choice)) return;
    a.href = choice.href;
    const label = a.querySelector<HTMLElement>('[data-download-label]');
    if (label) label.textContent = `Download for ${os}`;
    const note = document.getElementById(a.dataset.noteId ?? '');
    if (note) note.textContent = choice.note;
  });
}

export function enhanceDownloads(root: ParentNode = document) {
  const os = detectOS();
  if (!os) return;
  applyChoice(root, os, os);
  root.querySelectorAll<HTMLElement>(`[data-platform="${os}"]`).forEach((el) => el.setAttribute('data-detected', ''));
  if (os === 'macOS') void isIntelMac().then((intel) => intel && applyChoice(root, 'macOSIntel', os));
}
