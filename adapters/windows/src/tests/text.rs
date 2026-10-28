// Copyright 2026 The AccessKit Authors. All rights reserved.
// Licensed under the Apache License, Version 2.0 (found in
// the LICENSE-APACHE file) or the MIT license (found in
// the LICENSE-MIT file), at your option.

use accesskit::{
    Action, Node, NodeId, Role, TextPosition, TextSelection, TreeId, TreeInfo, TreeUpdate,
};
use accesskit_consumer::Tree;
use std::sync::Arc;
use windows::{
    Win32::{Foundation::*, UI::Accessibility::*},
    core::*,
};

use crate::context::{ActionHandlerWrapper, Context};

struct IgnoreChanges;

impl accesskit_consumer::TreeChangeHandler for IgnoreChanges {
    fn node_added(&mut self, _: &accesskit_consumer::NodeRef) {}
    fn node_updated(&mut self, _: &accesskit_consumer::NodeRef, _: &accesskit_consumer::NodeRef) {}
    fn focus_moved(
        &mut self,
        _: Option<&accesskit_consumer::NodeRef>,
        _: Option<&accesskit_consumer::NodeRef>,
    ) {
    }
    fn node_removed(&mut self, _: &accesskit_consumer::NodeRef) {}
}

const EDITOR: NodeId = NodeId(0);
const RUN: NodeId = NodeId(1);

fn state(selection: Option<(usize, usize)>, text: &str, lengths: &[u8]) -> TreeUpdate {
    let mut editor = Node::new(Role::MultilineTextInput);
    editor.add_action(Action::Focus);
    editor.set_children([RUN]);
    if let Some((anchor, focus)) = selection {
        editor.set_text_selection(TextSelection {
            anchor: TextPosition {
                node: RUN,
                character_index: anchor,
            },
            focus: TextPosition {
                node: RUN,
                character_index: focus,
            },
        });
    }
    let mut run = Node::new(Role::TextRun);
    run.set_value(text);
    run.set_character_lengths(lengths);
    TreeUpdate {
        nodes: vec![(EDITOR, editor), (RUN, run)],
        tree: Some(TreeInfo::new(EDITOR)),
        tree_id: TreeId::ROOT,
        focus: EDITOR,
    }
}

fn context(selection: Option<(usize, usize)>, focused: bool) -> Arc<Context> {
    Context::new(
        HWND::default().into(),
        Tree::new(state(selection, "a😀e\u{301}中", &[1, 4, 3, 3]), focused),
        Arc::new(ActionHandlerWrapper::new(super::simple::NullActionHandler)),
        false,
    )
}

fn provider(context: &Arc<Context>) -> IRawElementProviderSimple {
    let id = context.read_tree().state().root_id();
    context.get_or_create_platform_node(id).into_interface()
}

#[test]
fn text_pattern2_discovery() -> Result<()> {
    let context = context(Some((0, 2)), false);
    let provider = provider(&context);
    let pattern = unsafe { provider.GetPatternProvider(UIA_TextPattern2Id) }?;
    let _: ITextProvider2 = pattern.cast()?;
    let _: ITextProvider = pattern.cast()?;
    let available = unsafe { provider.GetPropertyValue(UIA_IsTextPattern2AvailablePropertyId) }?;
    assert!(unsafe { windows::Win32::System::Variant::VariantToBoolean(&available) }?.as_bool());
    let _: ITextProvider = unsafe { provider.GetPatternProvider(UIA_TextPatternId) }?.cast()?;
    Ok(())
}

fn text_pattern2(context: &Arc<Context>) -> Result<ITextProvider2> {
    unsafe { provider(context).GetPatternProvider(UIA_TextPattern2Id) }?.cast()
}

#[test]
fn non_text_node_does_not_advertise_text_pattern2() -> Result<()> {
    let context = context(Some((0, 0)), false);
    let mut update = state(None, "", &[]);
    update.nodes[0].1 = Node::new(Role::Button);
    update.nodes.truncate(1);
    *context.tree.write().unwrap() = Tree::new(update, false);
    let provider = provider(&context);
    assert!(unsafe { provider.GetPatternProvider(UIA_TextPattern2Id) }.is_err());
    let available = unsafe { provider.GetPropertyValue(UIA_IsTextPattern2AvailablePropertyId) }?;
    assert!(!unsafe { windows::Win32::System::Variant::VariantToBoolean(&available) }?.as_bool());
    Ok(())
}

#[test]
fn caret_tracks_host_focus_changes() -> Result<()> {
    let context = context(Some((1, 1)), false);
    let pattern = text_pattern2(&context)?;
    for focused in [true, false, true, false] {
        context
            .tree
            .write()
            .unwrap()
            .update_host_focus_state_and_process_changes(focused, &mut IgnoreChanges);
        let mut active = BOOL::from(!focused);
        let _caret = unsafe { pattern.GetCaretRange(&mut active) }?;
        assert_eq!(active.as_bool(), focused);
    }
    Ok(())
}

#[test]
fn caret_uses_selection_focus() -> Result<()> {
    for (selection, prefix) in [((0, 2), "a😀"), ((3, 1), "a"), ((2, 2), "a😀")] {
        for focused in [false, true] {
            let context = context(Some(selection), focused);
            let pattern = text_pattern2(&context)?;
            let mut active = BOOL::default();
            let caret = unsafe { pattern.GetCaretRange(&mut active) }?;
            assert_eq!(active.as_bool(), focused);
            assert_eq!(unsafe { caret.GetText(-1) }?.to_string(), "");
            assert_eq!(
                unsafe {
                    caret.CompareEndpoints(
                        TextPatternRangeEndpoint_Start,
                        &caret,
                        TextPatternRangeEndpoint_End,
                    )
                }?,
                0
            );
            let document = unsafe { pattern.DocumentRange() }?;
            assert_eq!(
                unsafe { document.GetText(-1) }?.to_string(),
                "a😀e\u{301}中"
            );
            unsafe {
                document.MoveEndpointByRange(
                    TextPatternRangeEndpoint_End,
                    &caret,
                    TextPatternRangeEndpoint_Start,
                )
            }?;
            assert_eq!(unsafe { document.GetText(-1) }?.to_string(), prefix);
        }
    }
    Ok(())
}

#[test]
fn caret_without_selection_is_not_supported() -> Result<()> {
    let context = context(None, true);
    let pattern = text_pattern2(&context)?;
    let mut active = BOOL::from(true);
    let error = unsafe { pattern.GetCaretRange(&mut active) }.unwrap_err();
    assert_eq!(error.code(), HRESULT(UIA_E_NOTSUPPORTED as i32));
    assert!(!active.as_bool());
    assert_eq!(
        unsafe { pattern.GetCaretRange(std::ptr::null_mut()) }
            .unwrap_err()
            .code(),
        E_POINTER
    );
    assert_eq!(
        unsafe { pattern.RangeFromAnnotation(&provider(&context)) }
            .unwrap_err()
            .code(),
        HRESULT(UIA_E_NOTSUPPORTED as i32)
    );
    Ok(())
}

#[test]
fn empty_document_caret() -> Result<()> {
    let context = context(Some((0, 0)), false);
    *context.tree.write().unwrap() = Tree::new(state(Some((0, 0)), "", &[]), false);
    let pattern = text_pattern2(&context)?;
    let mut active = BOOL::from(true);
    let caret = unsafe { pattern.GetCaretRange(&mut active) }?;
    assert!(!active.as_bool());
    assert_eq!(unsafe { caret.GetText(-1) }?.to_string(), "");
    assert!(unsafe { caret.Compare(&pattern.DocumentRange()?) }?.as_bool());
    Ok(())
}

#[test]
fn expired_context_invalidates_caret_and_provider() -> Result<()> {
    let context = context(Some((1, 1)), true);
    let pattern = text_pattern2(&context)?;
    let mut active = BOOL::default();
    let caret = unsafe { pattern.GetCaretRange(&mut active) }?;
    drop(context);
    assert_eq!(
        unsafe { pattern.GetCaretRange(&mut active) }
            .unwrap_err()
            .code(),
        HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)
    );
    assert!(!active.as_bool());
    assert_eq!(
        unsafe { caret.GetText(-1) }.unwrap_err().code(),
        HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)
    );
    Ok(())
}

#[test]
fn removed_text_invalidates_caret() -> Result<()> {
    let context = context(Some((1, 1)), true);
    let pattern = text_pattern2(&context)?;
    let mut active = BOOL::default();
    let caret = unsafe { pattern.GetCaretRange(&mut active) }?;
    let update = TreeUpdate {
        nodes: vec![(NodeId(2), Node::new(Role::Window))],
        tree: Some(TreeInfo::new(NodeId(2))),
        tree_id: TreeId::ROOT,
        focus: NodeId(2),
    };
    context
        .tree
        .write()
        .unwrap()
        .update_and_process_changes(update, &mut IgnoreChanges);
    assert_eq!(
        unsafe { pattern.GetCaretRange(&mut active) }
            .unwrap_err()
            .code(),
        HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)
    );
    assert!(!active.as_bool());
    assert_eq!(
        unsafe { caret.GetText(-1) }.unwrap_err().code(),
        HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)
    );
    Ok(())
}

struct TextActivationHandler;

impl accesskit::ActivationHandler for TextActivationHandler {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        let mut update = state(Some((3, 1)), "a😀e\u{301}中", &[1, 4, 3, 3]);
        let run = &mut update.nodes[1].1;
        run.set_bounds(accesskit::Rect::new(10.0, 20.0, 60.0, 40.0));
        run.set_text_direction(accesskit::TextDirection::LeftToRight);
        run.set_character_positions([0.0, 10.0, 30.0, 40.0]);
        run.set_character_widths([10.0, 20.0, 10.0, 10.0]);
        Some(update)
    }
}

#[test]
fn native_text_pattern2_inactive() -> Result<()> {
    super::scope(
        "TextPattern2 test",
        TextActivationHandler,
        super::simple::NullActionHandler,
        |scope| {
            let element = unsafe { scope.uia.ElementFromHandle(scope.window.0) }?;
            let available =
                unsafe { element.GetCurrentPropertyValue(UIA_IsTextPattern2AvailablePropertyId) }?;
            assert!(
                unsafe { windows::Win32::System::Variant::VariantToBoolean(&available) }?.as_bool()
            );
            let pattern: IUIAutomationTextPattern2 =
                unsafe { element.GetCurrentPattern(UIA_TextPattern2Id) }?.cast()?;
            let selection = unsafe { pattern.GetSelection() }?;
            assert_eq!(unsafe { selection.Length() }?, 1);
            assert_eq!(
                unsafe { selection.GetElement(0)?.GetText(-1) }?.to_string(),
                "😀e\u{301}"
            );
            let mut active = BOOL::from(true);
            let caret = unsafe { pattern.GetCaretRange(&mut active) }?;
            assert!(!active.as_bool());
            assert_eq!(unsafe { caret.GetText(-1) }?.to_string(), "");
            let rectangles = unsafe { caret.GetBoundingRectangles() }?;
            let _rectangles_guard = scopeguard::guard(rectangles, |array| unsafe {
                windows::Win32::System::Ole::SafeArrayDestroy(array).unwrap();
            });
            assert_eq!(
                unsafe { windows::Win32::System::Ole::SafeArrayGetUBound(rectangles, 1) }?,
                3
            );
            let mut width = 1.0_f64;
            let mut height = 0.0_f64;
            unsafe {
                windows::Win32::System::Ole::SafeArrayGetElement(
                    rectangles,
                    &2,
                    (&mut width as *mut f64).cast(),
                )?;
                windows::Win32::System::Ole::SafeArrayGetElement(
                    rectangles,
                    &3,
                    (&mut height as *mut f64).cast(),
                )?;
            }
            assert_eq!(width, 0.0);
            assert_eq!(height, 20.0);
            let document = unsafe { pattern.DocumentRange() }?;
            unsafe {
                document.MoveEndpointByRange(
                    TextPatternRangeEndpoint_End,
                    &caret,
                    TextPatternRangeEndpoint_Start,
                )
            }?;
            assert_eq!(unsafe { document.GetText(-1) }?.to_string(), "a");
            Ok(())
        },
    )
}
