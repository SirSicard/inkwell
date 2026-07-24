# Security Policy

## Supported Versions

| Version | Supported |
| ------- | --------- |
| 0.1.x   | Yes       |

## Reporting a Vulnerability

Please report security vulnerabilities by emailing **mattias@aitappers.io**.

**Do NOT open a public issue for security vulnerabilities.**

We will:
- Acknowledge receipt within 48 hours
- Provide a timeline for a fix within 7 days
- Credit you in the release notes (unless you prefer anonymity)

## Scope

Inkwell runs locally and processes audio on your machine. The attack surface is limited, but we take the following seriously:

- **AI Polish proxy** (inkwell-worker): any vulnerability that could leak user text or bypass rate limits
- **Auto-updater**: any vulnerability that could serve malicious updates
- **File transcription**: any input that could cause code execution via crafted audio/video files
- **Voice Agent mode**: any vulnerability in the OpenClaw gateway communication
