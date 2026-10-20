"""Tests for opening encrypted PDFs through the binding."""

from __future__ import annotations

import hashlib
from typing import TYPE_CHECKING

import pytest

import pawdf

from .test_document import _pdf

if TYPE_CHECKING:
    from pathlib import Path

P_BITS = 0xFFFFF0C0


def _r5_pdf(user: bytes, owner: bytes, p: int = -3904) -> bytes:
    """A one-page R5 file whose strings and streams are not encrypted (Identity filters).

    At R5, `/U` and `/O` are plain SHA-256, so they can be built with hashlib alone.
    """
    vs, ks, ovs, oks = b'\x01' * 8, b'\x02' * 8, b'\x03' * 8, b'\x04' * 8
    u = hashlib.sha256(user + vs).digest() + vs + ks
    o = hashlib.sha256(owner + ovs + u).digest() + ovs + oks
    zeros = (b'\x00' * 32).hex().encode()
    encrypt = (
        b'<< /Filter /Standard /V 5 /R 5 /Length 256 /P %d /O <%s> /U <%s> /OE <%s> /UE <%s> '
        b'/CF << /StdCF << /CFM /AESV3 >> >> /StmF /Identity /StrF /Identity >>'
    ) % (p, o.hex().encode(), u.hex().encode(), zeros, zeros)
    return _pdf(b'', extra=(encrypt,), trailer_extra=b'/Encrypt 6 0 R')


def test_empty_user_password_opens() -> None:
    """A file with an empty user password opens without one and reports its encryption."""
    doc = pawdf.Document(_r5_pdf(b'', b'own'))
    enc = doc.encryption
    assert enc is not None
    assert (enc.revision, enc.method, enc.key_bits) == (5, 'identity', 256)
    assert enc.authenticated_as == 'user'
    assert enc.permissions.bits == P_BITS
    assert not enc.permissions.print
    assert not enc.permissions.fill_forms
    assert repr(enc.permissions) == 'Permissions(bits=0xfffff0c0)'
    assert repr(enc) == "Encryption(revision=5, method='identity', key_bits=256, authenticated_as='user')"


def test_permission_flags() -> None:
    """Each Table 22 flag is reported, not enforced."""
    perms = pawdf.Document(_r5_pdf(b'', b'own', p=-4)).encryption
    assert perms is not None
    p = perms.permissions
    flags = [
        p.print,
        p.modify,
        p.copy,
        p.annotate,
        p.fill_forms,
        p.extract_for_accessibility,
        p.assemble,
        p.print_high_quality,
    ]
    assert all(flags)


def test_passwords() -> None:
    """A user or owner password opens the file; none or a wrong one raises PasswordError."""
    data = _r5_pdf(b'usr', b'own')
    with pytest.raises(pawdf.PasswordError, match='a password is required'):
        pawdf.Document(data)
    with pytest.raises(pawdf.PasswordError, match='incorrect password'):
        pawdf.Document(data, password='nope')
    user = pawdf.Document(data, password='usr').encryption
    assert user is not None
    assert user.authenticated_as == 'user'
    owner = pawdf.Document(data, 'own').encryption
    assert owner is not None
    assert owner.authenticated_as == 'owner'


def test_open_passes_password(tmp_path: Path) -> None:
    """pawdf.open forwards the password."""
    p = tmp_path / 'enc.pdf'
    p.write_bytes(_r5_pdf(b'usr', b'own'))
    assert len(pawdf.open(p, password='usr')) == 1


def test_unsupported_and_hierarchy() -> None:
    """Unsupported handlers are named; both new errors are EncryptedErrors."""
    with pytest.raises(pawdf.UnsupportedEncryptionError, match=r'Adobe\.PubSec'):
        pawdf.Document(_pdf(b'', trailer_extra=b'/Encrypt << /Filter /Adobe.PubSec /V 4 /R 4 >>'))
    assert issubclass(pawdf.PasswordError, pawdf.EncryptedError)
    assert issubclass(pawdf.UnsupportedEncryptionError, pawdf.EncryptedError)


def test_unencrypted_has_no_encryption() -> None:
    """An unencrypted file has no encryption info."""
    assert pawdf.Document(_pdf(b'')).encryption is None
