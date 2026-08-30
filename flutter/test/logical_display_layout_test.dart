import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_hbb/common/logical_display_layout.dart';

void main() {
  test('linux peers always use logical layout', () {
    expect(
        useLogicalDisplayLayout(isPeerLinux: true, isPeerMacOS: false), isTrue);
  });

  test('macOS peers always use logical layout, whatever the display scales',
      () {
    // Every display scaled — the case the old scale-mix gate could not
    // handle, now covered by per-display shim arming (src/retina_shim.rs).
    expect(
        useLogicalDisplayLayout(isPeerLinux: false, isPeerMacOS: true), isTrue);
  });

  test('other peers keep raw DisplayInfo units', () {
    expect(useLogicalDisplayLayout(isPeerLinux: false, isPeerMacOS: false),
        isFalse);
  });
}
