"""Tests for _version.py."""

from __future__ import annotations

import importlib
from importlib.metadata import PackageNotFoundError, version
from typing import TYPE_CHECKING

from pawdf import _version

if TYPE_CHECKING:
    from pytest_mock import MockerFixture


def test_version_reads_installed_package_metadata() -> None:
    """`__version__` matches the installed distribution's own metadata version."""
    assert _version.__version__ == version('pawdf')


def test_version_falls_back_when_package_metadata_is_missing(mocker: MockerFixture) -> None:
    """An uninstalled/unregistered package falls back to a placeholder instead of raising."""
    mocker.patch('importlib.metadata.version', side_effect=PackageNotFoundError)

    reloaded = importlib.reload(_version)

    try:
        assert reloaded.__version__ == '0+unknown'
    finally:
        importlib.reload(_version)  # restore the real value for any test that runs after this one
