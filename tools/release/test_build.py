"""Checks that release dependency inspection rejects dynamic/external runtimes."""
import unittest
from build import check_windows_binaries, check_windows_imports, windows_imports


class WindowsDependencies(unittest.TestCase):
    def test_parse_dumpbin(self):
        self.assertEqual(windows_imports('    KERNEL32.dll\n    USER32.dll\nSummary\n'),
                         {'KERNEL32.dll', 'USER32.dll'})

    def test_system_imports(self):
        check_windows_imports({'KERNEL32.dll', 'bcryptprimitives.dll',
                               'BCRYPTPRIMITIVES.DLL', 'combase.dll', 'ComBase.dll', 'api-ms-win-core-synch-l1-2-0.dll'})

    def test_dynamic_runtime_rejected(self):
        for dll in ('VCRUNTIME140.dll', 'MSVCP140.dll', 'ucrtbase.dll',
                    'api-ms-win-crt-runtime-l1-1-0.dll'):
            with self.subTest(dll=dll), self.assertRaises(RuntimeError):
                check_windows_imports({dll})

    def test_all_rejected_dependencies_and_binaries_are_reported(self):
        with self.assertRaises(RuntimeError) as failure:
            check_windows_binaries([
                ('plugin.clap', {'unknown.dll', 'VCRUNTIME140.dll'}),
                ('standalone.exe', {'another.dll'}),
            ])
        for expected in ('plugin.clap', 'standalone.exe', 'unknown.dll',
                         'VCRUNTIME140.dll', 'another.dll'):
            self.assertIn(expected, str(failure.exception))

    def test_external_runtime_rejected(self):
        with self.assertRaises(RuntimeError):
            check_windows_imports({'libgcc_s_seh-1.dll'})


if __name__ == '__main__':
    unittest.main()
