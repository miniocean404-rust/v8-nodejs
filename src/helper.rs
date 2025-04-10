#[macro_export]
macro_rules! v8_string {
    ($scope:expr, $value:expr) => {
        v8::String::new($scope, $value).unwrap()
    };
}
