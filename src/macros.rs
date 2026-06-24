#[macro_export]
macro_rules! cuda_check {
    ($expr:expr, $msg:expr) => {{
        let err = unsafe { $expr };
        if err != 0 {
            log::error!("❌ CUDA ERROR {} at {}", err, $msg);
        } else {
            // log::debug!("✅ CUDA OK: {}", $msg);
        }
    }};
}

#[macro_export]
macro_rules! cuda_error {
    ($expr:expr) => {{
        let err = unsafe { $expr };
        if err != 0 {
            log::error!("❌ CUDA ERROR {}", err);
            Err(anyhow!("Cuda Operation Error: {err}"))
        } else {
            Ok(())
        }
    }};
}

#[macro_export]
macro_rules! npp_error {
    ($expr:expr) => {{
        let err = unsafe { $expr };
        if err != 0 {
            log::error!("❌ NPP ERROR {}", err);
            Err(anyhow!("Npp Operation Error: {err}"))
        } else {
            Ok(())
        }
    }};
}

#[macro_export]
macro_rules! elogger {
    ( $x:expr ) => {{
        // $x trackable::track_any_err!(
        $x.map_err(|e| {
            let module = module_path!();
            let file = file!();
            let line = line!();
            log::error!("module: {module} -> file: {file} -> line: {line}");
            log::error!("error: {:?}", e);
            // log::error!("error: {}", e.to_string());
            e
        })
        // XError::logger($x)
    }};
}

#[macro_export]
macro_rules! select_loop {
    // 基本用法：select_loop!(rx1 => handler1, rx2 => handler2)
    ($($receiver:ident => $handler:expr),+ $(,)?) => {{
        use crossbeam::channel::Select;

        loop {
            let mut sel = Select::new();
            $(
                let oper = sel.recv($receiver);
            )+

            let oper = sel.select();
            let index = oper.index();

            match index {
                $(
                    #[allow(non_upper_case_globals)]
                    i if i == { static mut IDX: usize = 0; unsafe { IDX } } => {
                        let data = oper.recv($receiver).unwrap();
                        $handler(data);
                    }
                )+
                _ => unreachable!(),
            }

            // 可以在这里添加退出条件
            // if should_exit { break; }
        }
    }};
}
