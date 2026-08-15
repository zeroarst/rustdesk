import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_hbb/common/logical_display_layout.dart';

void main() {
  test('linux peers always use logical layout', () {
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: true, isPeerMacOS: false, allDisplayScales: [2.0]),
        true);
  });
  test('other peers never use logical layout', () {
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: false,
            isPeerMacOS: false,
            allDisplayScales: [1.0, 2.0]),
        false);
  });
  test('mac with a scale-1 display uses logical layout (parking possible)', () {
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: false,
            isPeerMacOS: true,
            allDisplayScales: [1.0, 1.0, 2.0]),
        true);
  });
  test('mac with all displays scaled keeps legacy info-space (no parking)', () {
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: false, isPeerMacOS: true, allDisplayScales: [2.0]),
        false);
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: false,
            isPeerMacOS: true,
            allDisplayScales: [2.0, 2.0]),
        false);
  });
  test('mac all-1x is logical (divide is a harmless no-op)', () {
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: false, isPeerMacOS: true, allDisplayScales: [1.0]),
        true);
  });
  test('legacy scale-0 reports count as unscaled', () {
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: false,
            isPeerMacOS: true,
            allDisplayScales: [0.0, 2.0]),
        true);
  });
  test('empty display list is never logical', () {
    expect(
        useLogicalDisplayLayout(
            isPeerLinux: false, isPeerMacOS: true, allDisplayScales: []),
        false);
  });
}
