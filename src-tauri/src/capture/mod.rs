pub mod tonemap;

use std::mem::size_of;
use windows::core::Interface;
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SDR_WHITE_LEVEL,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput1, IDXGIResource,
    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTPUT_DESC,
};

pub struct CapturedMonitor {
    pub width: u32,
    pub height: u32,
    /// Tone-mapped, display-correct sRGB pixels, row-major, 4 bytes/pixel.
    pub rgba: Vec<u8>,
}

/// Captures every active monitor. Desktop Duplication hands back either
/// already-correct 8-bit BGRA (the desktop compositor did the SDR/HDR
/// composition itself) or, on some systems, raw linear scRGB FP16 — in
/// which case we tone-map it ourselves against the monitor's real SDR
/// white level so SDR content isn't blown out.
pub fn capture_all_monitors() -> windows::core::Result<Vec<CapturedMonitor>> {
    let mut results = Vec::new();
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;

        let mut adapter_index = 0u32;
        while let Ok(adapter) = factory.EnumAdapters1(adapter_index) {
            adapter_index += 1;

            let mut output_index = 0u32;
            while let Ok(output) = adapter.EnumOutputs(output_index) {
                output_index += 1;
                if let Ok(captured) = capture_output(&adapter, &output) {
                    results.push(captured);
                }
            }
        }
    }
    Ok(results)
}

unsafe fn capture_output(
    adapter: &IDXGIAdapter1,
    output: &IDXGIOutput,
) -> windows::core::Result<CapturedMonitor> {
    let desc: DXGI_OUTPUT_DESC = output.GetDesc()?;

    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    D3D11CreateDevice(
        adapter,
        D3D_DRIVER_TYPE_UNKNOWN,
        HMODULE::default(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        None,
        D3D11_SDK_VERSION,
        Some(&mut device),
        None,
        Some(&mut context),
    )?;
    let device = device.ok_or(windows::core::Error::from(
        windows::Win32::Foundation::E_FAIL,
    ))?;
    let context = context.ok_or(windows::core::Error::from(
        windows::Win32::Foundation::E_FAIL,
    ))?;

    let output1: IDXGIOutput1 = output.cast()?;
    let duplication = output1.DuplicateOutput(&device)?;

    let dup_desc = duplication.GetDesc();
    let width = dup_desc.ModeDesc.Width;
    let height = dup_desc.ModeDesc.Height;

    // The very first frame after (re)starting duplication can come back
    // blank on some drivers; keep acquiring (releasing in between) until we
    // get a frame with real accumulated content, or give up.
    let mut resource: Option<IDXGIResource> = None;
    let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
    let mut attempts = 0;
    loop {
        match duplication.AcquireNextFrame(200, &mut frame_info, &mut resource) {
            Ok(()) => {
                if frame_info.AccumulatedFrames > 0 || attempts >= 9 {
                    break;
                }
                duplication.ReleaseFrame()?;
                resource = None;
                attempts += 1;
                continue;
            }
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT && attempts < 9 => {
                attempts += 1;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    let resource = resource.ok_or(windows::core::Error::from(
        windows::Win32::Foundation::E_FAIL,
    ))?;
    let texture: ID3D11Texture2D = resource.cast()?;

    let mut tex_desc = D3D11_TEXTURE2D_DESC::default();
    texture.GetDesc(&mut tex_desc);
    tex_desc.Usage = D3D11_USAGE_STAGING;
    tex_desc.BindFlags = 0;
    tex_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    tex_desc.MiscFlags = 0;

    let mut staging: Option<ID3D11Texture2D> = None;
    device.CreateTexture2D(&tex_desc, None, Some(&mut staging))?;
    let staging = staging.ok_or(windows::core::Error::from(
        windows::Win32::Foundation::E_FAIL,
    ))?;

    context.CopyResource(&staging, &texture);

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
    let format = tex_desc.Format;

    let sdr_scale = sdr_white_scale_for_output(&desc).unwrap_or(1.0);

    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let row_pitch = mapped.RowPitch as usize;
    let data = std::slice::from_raw_parts(mapped.pData as *const u8, row_pitch * height as usize);

    match format {
        DXGI_FORMAT_R16G16B16A16_FLOAT => {
            for y in 0..height as usize {
                let row = &data[y * row_pitch..y * row_pitch + width as usize * 8];
                for x in 0..width as usize {
                    let px = &row[x * 8..x * 8 + 8];
                    let r = half::f16::from_bits(u16::from_le_bytes([px[0], px[1]])).to_f32();
                    let g = half::f16::from_bits(u16::from_le_bytes([px[2], px[3]])).to_f32();
                    let b = half::f16::from_bits(u16::from_le_bytes([px[4], px[5]])).to_f32();
                    let [r, g, b] = tonemap::tonemap_pixel([r, g, b], sdr_scale);
                    let out = (y * width as usize + x) * 4;
                    rgba[out] = tonemap::linear_to_srgb_u8(r);
                    rgba[out + 1] = tonemap::linear_to_srgb_u8(g);
                    rgba[out + 2] = tonemap::linear_to_srgb_u8(b);
                    rgba[out + 3] = 255;
                }
            }
        }
        DXGI_FORMAT_B8G8R8A8_UNORM => {
            for y in 0..height as usize {
                let row = &data[y * row_pitch..y * row_pitch + width as usize * 4];
                for x in 0..width as usize {
                    let px = &row[x * 4..x * 4 + 4];
                    let out = (y * width as usize + x) * 4;
                    rgba[out] = px[2];
                    rgba[out + 1] = px[1];
                    rgba[out + 2] = px[0];
                    rgba[out + 3] = 255;
                }
            }
        }
        _ => {
            return Err(windows::core::Error::from(
                windows::Win32::Foundation::E_FAIL,
            ))
        }
    }

    context.Unmap(&staging, 0);
    duplication.ReleaseFrame()?;

    Ok(CapturedMonitor {
        width,
        height,
        rgba,
    })
}

/// Looks up the real SDR white level for the monitor behind `desc` and
/// returns the scale factor that normalizes scRGB values against it.
/// Falls back to `Ok(1.0)` semantics (via caller's `unwrap_or`) if the
/// monitor can't be matched or the OS doesn't report a level for it.
unsafe fn sdr_white_scale_for_output(desc: &DXGI_OUTPUT_DESC) -> windows::core::Result<f32> {
    let device_name = pwstr_to_string(&desc.DeviceName);

    let mut path_count = 0u32;
    let mut mode_count = 0u32;
    if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count).0 != 0 {
        return Ok(1.0);
    }

    let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = vec![Default::default(); path_count as usize];
    let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = vec![Default::default(); mode_count as usize];
    if QueryDisplayConfig(
        QDC_ONLY_ACTIVE_PATHS,
        &mut path_count,
        paths.as_mut_ptr(),
        &mut mode_count,
        modes.as_mut_ptr(),
        None,
    )
    .0 != 0
    {
        return Ok(1.0);
    }
    paths.truncate(path_count as usize);

    for path in &paths {
        let mut source_name = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        source_name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        source_name.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        source_name.header.adapterId = path.sourceInfo.adapterId;
        source_name.header.id = path.sourceInfo.id;
        if DisplayConfigGetDeviceInfo(&mut source_name.header) != 0 {
            continue;
        }
        if pwstr_to_string(&source_name.viewGdiDeviceName) != device_name {
            continue;
        }

        let mut white_level = DISPLAYCONFIG_SDR_WHITE_LEVEL::default();
        white_level.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL;
        white_level.header.size = size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32;
        white_level.header.adapterId = path.targetInfo.adapterId;
        white_level.header.id = path.targetInfo.id;
        if DisplayConfigGetDeviceInfo(&mut white_level.header) == 0 {
            return Ok(tonemap::sdr_scale_from_white_level(
                white_level.SDRWhiteLevel,
            ));
        }
    }

    Ok(1.0)
}

fn pwstr_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
