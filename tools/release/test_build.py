"""Checks that release dependency inspection rejects dynamic/external runtimes."""
import unittest
from build import check_windows_imports, windows_imports


class WindowsDependencies(unittest.TestCase):
    def test_parse_dumpbin(self):
        self.assertEqual(windows_imports('    KERNEL32.dll\n    USER32.dll\nSummary\n'),
                         {'KERNEL32.dll', 'USER32.dll'})

    def test_system_imports(self):
        check_windows_imports({'KERNEL32.dll', 'api-ms-win-core-synch-l1-2-0.dll'})

    def test_dynamic_runtime_rejected(self):
        for dll in ('VCRUNTIME140.dll', 'MSVCP140.dll', 'ucrtbase.dll',
                    'api-ms-win-crt-runtime-l1-1-0.dll'):
            with self.subTest(dll=dll), self.assertRaises(RuntimeError):
                check_windows_imports({dll})

    def test_external_runtime_rejected(self):
        with self.assertRaises(RuntimeError):
            check_windows_imports({'libgcc_s_seh-1.dll'})


if __name__ == '__main__':
    unittest.main()
