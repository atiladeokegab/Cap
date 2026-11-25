CONTROL = {"id", "label", "icon", "focusable"}


def main_window(driver, script=()):
    """The main window's controls in layout order, and one view per step."""
    result = driver("main_window", script=list(script))
    assert "controls" in result, f"cap-driver has no main_window op: {result}"
    for control in result["controls"]:
        missing = CONTROL - control.keys()
        assert not missing, f"main window control lacks {sorted(missing)}: {control}"
    return result


def test_a_labelled_teleprompter_button_sits_beside_the_record_control(driver):
    controls = main_window(driver)["controls"]
    ids = [c["id"] for c in controls]
    assert "record" in ids, f"no record control in {ids}"
    assert "teleprompter" in ids, f"no Teleprompter button in {ids}"
    button = controls[ids.index("teleprompter")]
    assert "Teleprompter" in button["label"], "the button has no text label"
    assert button["icon"], "the button has no icon"
    assert button["focusable"] is True, "the button is not reachable by keyboard"
    assert abs(ids.index("teleprompter") - ids.index("record")) == 1, (
        f"the Teleprompter button is not beside the record control: {ids}"
    )


def test_tab_from_the_record_control_reaches_the_button_and_enter_opens_it(driver):
    controls = main_window(driver)["controls"]
    tabs = [{"action": "key", "key": "Tab"} for _ in controls]
    back = [{"action": "key", "key": "Shift+Tab"} for _ in controls]
    focus = {"action": "focus", "control": "record"}
    for keys in (tabs, back):
        result = main_window(driver, [focus, *keys])
        focused = [view["focused"] for view in result["views"]]
        if "teleprompter" in focused:
            presses = focused.index("teleprompter")
            break
    else:
        raise AssertionError("Tab never focuses the Teleprompter button")
    enter = {"action": "key", "key": "Enter"}
    opened = main_window(driver, [focus, *keys[:presses], enter])["views"][-1]
    assert opened["teleprompter_open"] is True, "Enter did not open the teleprompter"
