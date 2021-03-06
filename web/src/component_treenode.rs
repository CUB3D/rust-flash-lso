use crate::{EditableValue, TreeNodePath};
use flash_lso::types::{Element, Value};
use std::ops::Deref;
use std::rc::Rc;
use yew::prelude::*;
use yew::{Component, ComponentLink, Html, Properties};
use yewtil::NeqAssign;

#[derive(Debug)]
pub enum Msg {
    Selection(EditableValue),
    Toggle,
    Edited(Value),
    ElementChange(Element),
    CustomElementChange(Element),
    CustomElementChangeStandard(Element),
}

pub struct TreeNode {
    props: Props,
    link: ComponentLink<Self>,
    expanded: bool,
    value: Value,
}

#[derive(Properties, Clone, PartialEq)]
pub struct Props {
    pub parent_path: TreeNodePath,
    pub name: String,
    pub value: Value,
    pub parent_callback: Callback<EditableValue>,
    pub selection: Option<EditableValue>,
    pub filter: String,
    #[prop_or(None)]
    pub element_callback: Option<Callback<Element>>,
}

impl Component for TreeNode {
    type Message = Msg;
    type Properties = Props;

    fn create(props: Self::Properties, link: ComponentLink<Self>) -> Self {
        let value = props.value.clone();
        Self {
            props,
            link,
            expanded: false,
            value,
        }
    }

    fn update(&mut self, msg: Self::Message) -> bool {
        log::info!("<TreeNode>@{}, MSG: {:?}", self.path().string(), msg);
        match msg {
            Msg::Selection(val) => {
                self.props.parent_callback.emit(val);
                true
            }
            Msg::Toggle => {
                self.expanded = !self.expanded;
                true
            }
            Msg::Edited(v) => {
                self.value = v.clone();
                if let Some(x) = &self.props.element_callback {
                    x.emit(Element::new(self.props.name.clone(), v));
                }
                true
            }
            Msg::ElementChange(el) => {
                match &mut self.value {
                    Value::Object(old_el, _) => {
                        let index = old_el.iter().position(|e| e.name == el.name);
                        if let Some(index) = index {
                            old_el[index] = el;
                        }
                    }
                    _ => {
                        log::warn!("Unknown element change");
                    }
                }

                true
            }
            Msg::CustomElementChange(el) => {
                match &mut self.value {
                    Value::Custom(a, _b, _) => {
                        let index = a.iter().position(|e| e.name == el.name);
                        if let Some(index) = index {
                            a[index] = el;
                        }
                    }
                    _ => {
                        log::warn!("Unknown element change for custom element");
                    }
                }

                true
            }
            Msg::CustomElementChangeStandard(el) => {
                match &mut self.value {
                    Value::Custom(_a, b, _) => {
                        let index = b.iter().position(|e| e.name == el.name);
                        if let Some(index) = index {
                            b[index] = el;
                        }
                    }
                    _ => {
                        log::warn!("Unknown element change for custom element standard");
                    }
                }

                true
            }
        }
    }

    fn change(&mut self, props: Self::Properties) -> bool {
        self.props.neq_assign(props)
    }

    fn view(&self) -> Html {
        let name = self.props.name.clone();
        let value = self.value.clone();

        let icon = if TreeNode::has_children(&value) {
            if self.expanded {
                "icon/folder-minus.svg"
            } else {
                "icon/folder-plus.svg"
            }
        } else {
            "icon/file-text.svg"
        };

        let classes = if self.selected() {
            "text-white bg-primary rounded-pill pl-2 pr-2"
        } else {
            "pl-2 pr-2"
        };

        let callback = self.link.callback(Msg::Edited);
        let v = self.value.clone();
        let path = self.path();

        if !self.is_visible() {
            return html! {};
        }

        html! {
             <div>
                <span onclick=self.link.callback(|_| Msg::Toggle)>
                    <img src={icon} style={"width: 32; height: 32;"} class={"mr-2"}/>
                </span>
                <span
                    class={classes}
                    onclick=self.link.callback(move |_| Msg::Selection(EditableValue {
                        value: v.clone(),
                        callback: callback.clone(),
                        path: path.clone(),
                    }))>{ name }</span>
                { if self.expanded {
                    self.view_sol_value(Rc::new(self.value.clone()))
                } else {
                    html!{}
                }}
             </div>
        }
    }
}

impl TreeNode {
    pub fn is_visible(&self) -> bool {
        // Visible if no filter or if we are included in filter, also we must be visible if we have visible children
        let has_visible_children = match &self.props.value {
            Value::Object(ele, _) => ele.iter().any(|e| e.name.contains(&self.props.filter)),
            Value::ECMAArray(e1, e2, _) => {
                e2.iter().any(|e| e.name.contains(&self.props.filter))
                    || e1
                        .iter()
                        .enumerate()
                        .any(|(i, _e)| format!("{}", i).contains(&self.props.filter))
            }
            Value::StrictArray(e1) => e1
                .iter()
                .enumerate()
                .any(|(i, _e)| format!("{}", i).contains(&self.props.filter)),
            Value::VectorObject(e1, _, _) => e1
                .iter()
                .enumerate()
                .any(|(i, _e)| format!("{}", i).contains(&self.props.filter)),
            Value::Custom(e1, e2, _) => {
                e1.iter().any(|e| e.name.contains(&self.props.filter))
                    || e2.iter().any(|e| e.name.contains(&self.props.filter))
            }
            _ => false,
        };

        self.props.filter.is_empty()
            || self.props.name.contains(&self.props.filter)
            || (TreeNode::has_children(&self.props.value) && has_visible_children)
    }

    pub fn path(&self) -> TreeNodePath {
        self.props.parent_path.join(self.props.name.clone())
    }

    pub fn selected(&self) -> bool {
        let selected_path = self.props.selection.clone().map(|s| s.path);
        selected_path.map_or(false, |tnp| tnp.contains(self.path()))
    }

    pub fn has_children(data: &Value) -> bool {
        matches!(
            data,
            Value::Object(_, _)
                | Value::StrictArray(_)
                | Value::ECMAArray(_, _, _)
                | Value::VectorObject(_, _, _)
                | Value::AMF3(_)
                | Value::Dictionary(_, _)
                | Value::Custom(_, _, _)
        )
    }

    pub fn view_array_element(&self, index: usize, data: &Rc<Value>) -> Html {
        html! {
            <div>
                <TreeNode filter=self.props.filter.clone() selection=self.props.selection.clone() parent_path=self.path() name={format!("{}", index)} value={data.deref().clone()} parent_callback={self.link.callback(|val| Msg::Selection(val))}></TreeNode>
            </div>
        }
    }

    pub fn view_sol_value(&self, data: Rc<Value>) -> Html {
        match data.deref() {
            Value::AMF3(e) => self.view_sol_value(e.clone()),
            Value::Object(elements, _class_def) => html! {
                <ul>
                    { for elements.iter().map(|e| html! {
                        <TreeNode element_callback=self.link.callback(|el| Msg::ElementChange(el)) filter=self.props.filter.clone() selection=self.props.selection.clone() parent_path=self.path() name={e.name.clone()} value={e.value.deref().clone()} parent_callback={self.link.callback(|val| Msg::Selection(val))}></TreeNode>
                    })}
                </ul>
            },
            Value::StrictArray(x) => html! {
                <ul>
                    { for x.iter().enumerate().map(|(i, v)| self.view_array_element(i, v))}
                </ul>
            },
            Value::ECMAArray(dense, assoc, _size) => html! {
                    <ul>
                       { for dense.iter().enumerate().map(|(i, v)| self.view_array_element(i, v))}
                        { for assoc.iter().map(|e| html! {
                            <TreeNode filter=self.props.filter.clone() selection=self.props.selection.clone() parent_path=self.path() name={e.name.clone()} value={e.value.deref().clone()} parent_callback={self.link.callback(|val| Msg::Selection(val))}></TreeNode>
                        })}
                    </ul>
            },
            Value::VectorObject(children, _name, _fixed_len) => html! {
                <ul>
                   { for children.iter().enumerate().map(|(i, v)| self.view_array_element(i, v))}
                </ul>
            },
            Value::Dictionary(children, _) => html! {
                <ul>
                    { for children.iter().map(|(k, v)| html! {
                            <>
                            <li>
                                <TreeNode filter=self.props.filter.clone() selection=self.props.selection.clone() parent_path=self.path() name="key" value={k.deref().clone()} parent_callback=self.link.callback(|val| Msg::Selection(val))></TreeNode>
                            </li>
                            <li>
                                <TreeNode filter=self.props.filter.clone() selection=self.props.selection.clone() parent_path=self.path() name="value" value={v.deref().clone()} parent_callback=self.link.callback(|val| Msg::Selection(val))></TreeNode>
                            </li>
                            </>
                        })}
                </ul>
            },
            Value::Custom(el, el2, _class_def) => html! {
                <ul>
                    <li>
                        {"Custom elements"}
                        <ul>
                            { for el.iter().map(|e| html! {
                                <TreeNode element_callback=self.link.callback(|el| Msg::CustomElementChange(el)) filter=self.props.filter.clone() selection=self.props.selection.clone() parent_path=self.path() name={e.name.clone()} value={e.value.deref().clone()} parent_callback={self.link.callback(|val| Msg::Selection(val))}></TreeNode>
                            })}
                        </ul>
                    </li>
                    <li>
                        {"Standard elements"}
                        <ul>
                           { for el2.iter().map(|e| html! {
                                <TreeNode element_callback=self.link.callback(|el| Msg::CustomElementChangeStandard(el)) filter=self.props.filter.clone() selection=self.props.selection.clone() parent_path=self.path() name={e.name.clone()} value={e.value.deref().clone()} parent_callback={self.link.callback(|val| Msg::Selection(val))}></TreeNode>
                            })}
                        </ul>
                    </li>
                </ul>
            },
            _ => html! {},
        }
    }
}
