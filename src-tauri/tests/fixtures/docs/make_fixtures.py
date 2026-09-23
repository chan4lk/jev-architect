#!/usr/bin/env python3
"""Generates the docs.rs test fixtures in this directory, using only the
Python standard library (zipfile for DOCX; a hand-written minimal, valid
PDF 1.4 for the two PDF fixtures).

Run from anywhere:
    python3 make_fixtures.py

Regenerates (in this directory):
    sample.pdf      - two pages of Helvetica text (has a text layer)
    scanned.pdf     - one page with only a drawn rectangle (no text layer)
    sample.docx     - a few headed paragraphs (Heading1 / Heading2 styles)
    sample.md       - the same requirements doc, as Markdown (ATX headings)
    sample.txt      - the same requirements doc, as plain text (blank-line
                      separated blocks, no headings)
    large.md        - many requirement sections, comfortably over 20,000
                      estimated tokens (ceil(chars / 4)), to exercise the
                      pipeline's large-document path
"""

import os
import zipfile
from xml.sax.saxutils import escape as xml_escape

HERE = os.path.dirname(os.path.abspath(__file__))


# ---------------------------------------------------------------------
# Shared content: a short, realistic BISTEC requirements document for an
# insurer's customer self-service portal.
# ---------------------------------------------------------------------

SECTIONS = [
    (
        "Overview",
        [
            "This document captures the requirements for a new customer "
            "self-service portal for a regional insurer client of BISTEC "
            "Global.",
            "Policyholders will use the portal to view policy documents, "
            "file and track claims, and update contact and billing "
            "details without calling the contact centre.",
        ],
    ),
    (
        "Scope and Users",
        [
            "The portal must support approximately 20,000 registered "
            "policyholders at launch, with headroom for growth as the "
            "insurer migrates further policy lines onto the platform.",
            "Peak concurrent usage is expected around monthly billing "
            "cycles and renewal periods, so the system should handle "
            "bursty traffic without manual intervention.",
        ],
    ),
    (
        "Authentication",
        [
            "Corporate staff and agents will sign in with the insurer's "
            "existing Microsoft 365 tenant via single sign-on (SSO); "
            "policyholders will use a separate, portal-specific login.",
            "The authentication design should reuse BISTEC's default "
            "Microsoft identity platform integration rather than a "
            "bespoke identity provider.",
        ],
    ),
    (
        "Budget and Timeline",
        [
            "The client has stated a tight budget for this engagement "
            "and prefers to reuse managed cloud services over "
            "self-hosted infrastructure wherever the cost difference is "
            "significant.",
            "A first release is targeted within one quarter, covering "
            "policy viewing and claims status only; billing updates and "
            "document uploads can follow in a later phase.",
        ],
    ),
    (
        "Team and Delivery",
        [
            "The insurer's in-house engineering team is a .NET team "
            "with several years of experience running ASP.NET Core "
            "services in Azure, and would maintain the solution after "
            "handover.",
            "BISTEC will pair with the client team during delivery so "
            "that ownership can transfer smoothly at the end of the "
            "engagement.",
        ],
    ),
    (
        "Compliance",
        [
            "The portal processes personal and policy data for "
            "EU-resident customers, so the solution must meet GDPR "
            "requirements for data minimisation, consent, and the "
            "right to erasure.",
            "Audit logging of access to policy and claims records is "
            "required for the insurer's compliance reporting.",
        ],
    ),
    (
        "Non-Functional Requirements",
        [
            "The portal should remain available during the insurer's "
            "business hours with no planned downtime for routine "
            "deployments.",
            "Response times for common actions such as viewing a policy "
            "or checking claim status should stay under two seconds "
            "under normal load.",
        ],
    ),
]


def section_paragraphs(sentences):
    return " ".join(sentences)


# ---------------------------------------------------------------------
# sample.md / sample.txt
# ---------------------------------------------------------------------

def write_markdown():
    lines = ["# Customer Self-Service Portal — Requirements Overview", ""]
    for i, (heading, sentences) in enumerate(SECTIONS):
        level = "##" if i > 0 else "##"
        lines.append(f"{level} {heading}")
        lines.append("")
        lines.append(section_paragraphs(sentences))
        lines.append("")
    text = "\n".join(lines).rstrip() + "\n"
    with open(os.path.join(HERE, "sample.md"), "w", encoding="utf-8") as f:
        f.write(text)


def write_txt():
    blocks = ["Customer Self-Service Portal - Requirements Overview"]
    for heading, sentences in SECTIONS:
        blocks.append(heading.upper())
        blocks.append(section_paragraphs(sentences))
    text = "\n\n".join(blocks) + "\n"
    with open(os.path.join(HERE, "sample.txt"), "w", encoding="utf-8") as f:
        f.write(text)


# ---------------------------------------------------------------------
# sample.docx
# ---------------------------------------------------------------------

def docx_paragraph(text, style=None):
    run = f"<w:r><w:t xml:space=\"preserve\">{xml_escape(text)}</w:t></w:r>"
    if style:
        ppr = f"<w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr>"
        return f"<w:p>{ppr}{run}</w:p>"
    return f"<w:p>{run}</w:p>"


def build_document_xml():
    body_parts = [
        docx_paragraph(
            "Customer Self-Service Portal — Requirements Overview", "Title"
        )
    ]
    for i, (heading, sentences) in enumerate(SECTIONS):
        style = "Heading1" if i == 0 else "Heading2"
        body_parts.append(docx_paragraph(heading, style))
        for sentence in sentences:
            body_parts.append(docx_paragraph(sentence))
    body_parts.append('<w:sectPr/>')

    body = "".join(body_parts)
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
        f"<w:body>{body}</w:body>"
        "</w:document>"
    )


CONTENT_TYPES_XML = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
    '<Default Extension="xml" ContentType="application/xml"/>'
    '<Override PartName="/word/document.xml" '
    'ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
    "</Types>"
)

RELS_XML = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    '<Relationship Id="rId1" '
    'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" '
    'Target="word/document.xml"/>'
    "</Relationships>"
)

DOCUMENT_RELS_XML = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>'
)


def write_docx():
    path = os.path.join(HERE, "sample.docx")
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", CONTENT_TYPES_XML)
        z.writestr("_rels/.rels", RELS_XML)
        z.writestr("word/_rels/document.xml.rels", DOCUMENT_RELS_XML)
        z.writestr("word/document.xml", build_document_xml())


# ---------------------------------------------------------------------
# PDF: a minimal, hand-written, valid PDF 1.4 with a correct xref table.
# ---------------------------------------------------------------------

def pdf_sanitize(text):
    """PDF text here uses the base Helvetica font with no custom encoding,
    so keep it to plain ASCII (WinAnsi-safe)."""
    replacements = {
        "—": "-",  # em dash
        "–": "-",  # en dash
        "‘": "'",
        "’": "'",
        "“": '"',
        "”": '"',
    }
    for src, dst in replacements.items():
        text = text.replace(src, dst)
    return text.encode("ascii", "replace").decode("ascii")


def pdf_escape(text):
    text = pdf_sanitize(text)
    return text.replace("\\", r"\\").replace("(", r"\(").replace(")", r"\)")


def build_content_stream(lines, font_size=11, leading=15, start_x=72, start_y=740):
    ops = ["BT", f"/F1 {font_size} Tf", f"{leading} TL", f"{start_x} {start_y} Td"]
    for i, line in enumerate(lines):
        if i > 0:
            ops.append("T*")
        if line.strip():
            ops.append(f"({pdf_escape(line)}) Tj")
    ops.append("ET")
    return "\n".join(ops).encode("latin-1")


def wrap(text, width=90):
    words = text.split()
    lines = []
    current = ""
    for word in words:
        candidate = f"{current} {word}".strip()
        if len(candidate) > width and current:
            lines.append(current)
            current = word
        else:
            current = candidate
    if current:
        lines.append(current)
    return lines


def paginate(lines, per_page=48):
    return [lines[i : i + per_page] for i in range(0, len(lines), per_page)] or [[]]


def build_text_pdf(title, sections, path):
    lines = [title, ""]
    for heading, sentences in sections:
        lines.append(heading.upper())
        lines.extend(wrap(section_paragraphs(sentences)))
        lines.append("")

    pages_lines = paginate(lines)
    write_pdf(pages_lines, path)


def write_pdf(pages_lines, path, page_contents_override=None):
    """pages_lines: list of list-of-str (one list per page). If
    page_contents_override is given, it's used as the raw content stream
    bytes per page instead of building one from text lines (used for the
    no-text-layer fixture)."""

    obj_id = 1
    catalog_id = obj_id
    obj_id += 1
    pages_id = obj_id
    obj_id += 1
    font_id = obj_id
    obj_id += 1

    page_ids = []
    content_ids = []
    for _ in pages_lines:
        page_ids.append(obj_id)
        obj_id += 1
        content_ids.append(obj_id)
        obj_id += 1

    objects = {}
    objects[catalog_id] = f"<< /Type /Catalog /Pages {pages_id} 0 R >>".encode("latin-1")

    kids = " ".join(f"{pid} 0 R" for pid in page_ids)
    objects[pages_id] = (
        f"<< /Type /Pages /Kids [{kids}] /Count {len(page_ids)} >>".encode("latin-1")
    )

    objects[font_id] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"

    for i, page_id in enumerate(page_ids):
        content_id = content_ids[i]
        objects[page_id] = (
            f"<< /Type /Page /Parent {pages_id} 0 R "
            f"/MediaBox [0 0 612 792] "
            f"/Resources << /Font << /F1 {font_id} 0 R >> >> "
            f"/Contents {content_id} 0 R >>"
        ).encode("latin-1")

        if page_contents_override is not None:
            stream = page_contents_override[i]
        else:
            stream = build_content_stream(pages_lines[i])
        objects[content_id] = (
            f"<< /Length {len(stream)} >>\nstream\n".encode("latin-1")
            + stream
            + b"\nendstream"
        )

    max_id = obj_id - 1
    buf = bytearray()
    buf += b"%PDF-1.4\n"
    offsets = {}
    for oid in range(1, max_id + 1):
        offsets[oid] = len(buf)
        buf += f"{oid} 0 obj\n".encode("latin-1")
        buf += objects[oid]
        buf += b"\nendobj\n"

    xref_offset = len(buf)
    buf += f"xref\n0 {max_id + 1}\n".encode("latin-1")
    buf += b"0000000000 65535 f \n"
    for oid in range(1, max_id + 1):
        buf += f"{offsets[oid]:010d} 00000 n \n".encode("latin-1")

    buf += b"trailer\n"
    buf += f"<< /Size {max_id + 1} /Root {catalog_id} 0 R >>\n".encode("latin-1")
    buf += b"startxref\n"
    buf += f"{xref_offset}\n".encode("latin-1")
    buf += b"%%EOF"

    with open(path, "wb") as f:
        f.write(bytes(buf))


def write_scanned_pdf():
    """A single valid page with only a drawn rectangle — no text objects at
    all, so pdf-extract must return no text (NoTextLayer)."""
    rectangle_stream = b"1 0 0 RG\n2 w\n72 72 400 200 re\nS"
    write_pdf([[]], os.path.join(HERE, "scanned.pdf"), page_contents_override=[rectangle_stream])


# ---------------------------------------------------------------------
# large.md: comfortably over 20,000 estimated tokens (ceil(chars / 4)).
# ---------------------------------------------------------------------

LARGE_TOPICS = [
    "single sign-on against the insurer's Microsoft 365 tenant",
    "claims intake and status tracking for policyholders",
    "policy document retrieval and download",
    "GDPR-compliant handling of personal and policy data",
    "billing and payment history for registered users",
    "tight budget constraints favouring managed cloud services",
    "the insurer's existing .NET engineering team",
    "scaling to roughly 20,000 registered policyholders",
    "audit logging of access to claims and policy records",
    "notifications for claim status changes",
]


def build_large_markdown():
    lines = [
        "# Customer Self-Service Portal — Detailed Requirements",
        "",
        "This document expands the requirements overview into individual, "
        "numbered functional requirements for the insurer's customer "
        "self-service portal, grouped by theme.",
        "",
    ]

    req_num = 1
    for round_ in range(15):
        for topic in LARGE_TOPICS:
            lines.append(f"## FR-{req_num}: Requirement covering {topic}")
            lines.append("")
            paragraph = (
                f"Requirement FR-{req_num} addresses {topic}. "
                f"The portal must support this capability for all "
                f"registered users, with behaviour that is consistent "
                f"across desktop and mobile browsers. "
                f"Acceptance for this requirement depends on sign-off "
                f"from both the insurer's product owner and its "
                f"compliance team, since it touches customer data. "
                f"BISTEC's default stack should be used wherever it "
                f"satisfies this requirement, to keep the tight budget "
                f"and the .NET team's existing skills in mind. "
                f"Edge cases, error states, and audit trail expectations "
                f"for this requirement are captured in the accompanying "
                f"test plan, iteration {round_ + 1}."
            )
            lines.append(paragraph)
            lines.append("")
            req_num += 1

    return "\n".join(lines).rstrip() + "\n"


def write_large_markdown():
    text = build_large_markdown()
    with open(os.path.join(HERE, "large.md"), "w", encoding="utf-8") as f:
        f.write(text)
    chars = len(text)
    tokens = -(-chars // 4)
    print(f"large.md: {chars} chars, ~{tokens} estimated tokens")


def main():
    write_markdown()
    write_txt()
    write_docx()
    build_text_pdf(
        "Customer Self-Service Portal — Requirements Overview",
        SECTIONS,
        os.path.join(HERE, "sample.pdf"),
    )
    write_scanned_pdf()
    write_large_markdown()
    print("Fixtures written to", HERE)


if __name__ == "__main__":
    main()
