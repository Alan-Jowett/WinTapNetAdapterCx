#include "wintap.h"

static constexpr ULONG WINTAP_FRAME_MINIMUM = 14;
static constexpr ULONG WINTAP_FRAME_MAXIMUM = 1514;
static constexpr ULONG WINTAP_FRAME_LIMIT = 256;
static constexpr ULONG WINTAP_PENDING_READ_LIMIT = 256;
static constexpr ULONG WINTAP_PENDING_WRITE_LIMIT = 256;
static PWINTAP_DEVICE_CONTEXT g_ControlContext = nullptr;
static WDFDEVICE g_ControlDevice = nullptr;
static NETADAPTER g_Adapter = nullptr;
static KSPIN_LOCK g_ControlContextLock;

static BOOLEAN
WintapAcquireCallback(
    _In_ PWINTAP_DEVICE_CONTEXT Context
    );

static PWINTAP_DEVICE_CONTEXT
WintapAcquireControlCallback(
    VOID
    )
{
    PWINTAP_DEVICE_CONTEXT context = nullptr;
    KIRQL oldIrql;

    KeAcquireSpinLock(&g_ControlContextLock, &oldIrql);
    if (g_ControlContext != nullptr &&
        WintapAcquireCallback(g_ControlContext)) {
        context = g_ControlContext;
    }
    KeReleaseSpinLock(&g_ControlContextLock, oldIrql);
    return context;
}

static VOID
WintapClearControlContext(
    _In_ PWINTAP_DEVICE_CONTEXT Context
    )
{
    KIRQL oldIrql;

    KeAcquireSpinLock(&g_ControlContextLock, &oldIrql);
    if (g_ControlContext == Context) {
        g_ControlContext = nullptr;
    }
    KeReleaseSpinLock(&g_ControlContextLock, oldIrql);
}

static PWINTAP_DEVICE_CONTEXT
WintapPeekControlContext(
    VOID
    )
{
    PWINTAP_DEVICE_CONTEXT context;
    KIRQL oldIrql;

    KeAcquireSpinLock(&g_ControlContextLock, &oldIrql);
    context = g_ControlContext;
    KeReleaseSpinLock(&g_ControlContextLock, oldIrql);
    return context;
}

static BOOLEAN
WintapAcquireCallback(
    _In_ PWINTAP_DEVICE_CONTEXT Context
    )
{
    BOOLEAN acquired = FALSE;
    WdfSpinLockAcquire(Context->FrameLock);
    if (!Context->Closing && !Context->Removed && !Context->Suspended) {
        if (InterlockedIncrement(&Context->ActiveCallbacks) == 1) {
            KeClearEvent(&Context->CallbackIdle);
        }
        acquired = TRUE;
    }
    WdfSpinLockRelease(Context->FrameLock);
    return acquired;
}

static VOID
WintapReleaseCallback(
    _In_ PWINTAP_DEVICE_CONTEXT Context
    )
{
    if (InterlockedDecrement(&Context->ActiveCallbacks) == 0) {
        KeSetEvent(&Context->CallbackIdle, IO_NO_INCREMENT, FALSE);
    }
}

static VOID
WintapWaitForWriteDrain(
    _In_ PWINTAP_DEVICE_CONTEXT Context
    )
{
    BOOLEAN running;
    BOOLEAN queued;

    WdfSpinLockAcquire(Context->FrameLock);
    running = Context->WriteDrainRunning;
    queued = Context->WriteDrainQueued;
    WdfSpinLockRelease(Context->FrameLock);

    if (running || queued) {
        KeWaitForSingleObject(
            &Context->WriteDrainIdle,
            Executive,
            KernelMode,
            FALSE,
            nullptr);
    }
}

static VOID
WintapWaitForReadCompletion(
    _In_ PWINTAP_DEVICE_CONTEXT Context
    )
{
    BOOLEAN running;
    BOOLEAN queued;

    WdfSpinLockAcquire(Context->FrameLock);
    running = Context->ReadCompletionRunning;
    queued = Context->ReadCompletionQueued;
    WdfSpinLockRelease(Context->FrameLock);

    if (running || queued) {
        KeWaitForSingleObject(
            &Context->ReadCompletionIdle,
            Executive,
            KernelMode,
            FALSE,
            nullptr);
    }
}

static NTSTATUS
WintapMarkPendingRequest(
    _In_ PWINTAP_DEVICE_CONTEXT Context,
    _In_ WDFREQUEST Request,
    _In_ BOOLEAN IsRead
    )
{
    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(
        &attributes,
        WINTAP_REQUEST_CONTEXT);

    PWINTAP_REQUEST_CONTEXT requestContext = nullptr;
    NTSTATUS status = WdfObjectAllocateContext(
        Request,
        &attributes,
        reinterpret_cast<PVOID*>(&requestContext));
    if (status == STATUS_OBJECT_NAME_EXISTS) {
        requestContext = WintapGetRequestContext(Request);
        status = STATUS_SUCCESS;
    }
    if (!NT_SUCCESS(status)) {
        return status;
    }

    WdfSpinLockAcquire(Context->FrameLock);
    ULONG* count = IsRead
        ? &Context->PendingReadCount
        : &Context->PendingWriteCount;
    ULONG limit = IsRead
        ? Context->PendingReadLimit
        : Context->PendingWriteLimit;
    if (Context->Closing || Context->Removed || Context->Suspended ||
        requestContext->Counted || *count >= limit) {
        WdfSpinLockRelease(Context->FrameLock);
        return STATUS_DEVICE_BUSY;
    }

    requestContext->Counted = TRUE;
    requestContext->IsRead = IsRead;
    ++*count;
    WdfSpinLockRelease(Context->FrameLock);
    return STATUS_SUCCESS;
}

static VOID
WintapReleasePendingRequest(
    _In_ PWINTAP_DEVICE_CONTEXT Context,
    _In_ WDFREQUEST Request
    )
{
    PWINTAP_REQUEST_CONTEXT requestContext =
        WintapGetRequestContext(Request);

    WdfSpinLockAcquire(Context->FrameLock);
    if (requestContext->Counted) {
        ULONG* count = requestContext->IsRead
            ? &Context->PendingReadCount
            : &Context->PendingWriteCount;
        if (*count != 0) {
            --*count;
        }
        requestContext->Counted = FALSE;
    }
    WdfSpinLockRelease(Context->FrameLock);
}

static VOID
WintapFreeFrames(
    _In_ PWINTAP_DEVICE_CONTEXT Context
    )
{
    for (;;) {
        PWINTAP_FRAME frame = nullptr;

        WdfSpinLockAcquire(Context->FrameLock);
        if (!IsListEmpty(&Context->ToStackQueue)) {
            PLIST_ENTRY entry = RemoveHeadList(&Context->ToStackQueue);
            frame = CONTAINING_RECORD(entry, WINTAP_FRAME, ListEntry);
            --Context->ToStackCount;
        } else if (!IsListEmpty(&Context->FromStackQueue)) {
            PLIST_ENTRY entry = RemoveHeadList(&Context->FromStackQueue);
            frame = CONTAINING_RECORD(entry, WINTAP_FRAME, ListEntry);
            --Context->FromStackCount;
        }
        WdfSpinLockRelease(Context->FrameLock);

        if (frame == nullptr) {
            return;
        }

        ExFreePoolWithTag(frame, 'paTW');
    }
}

static NTSTATUS
WintapQueueFrame(
    _In_ PWINTAP_DEVICE_CONTEXT Context,
    _In_ PLIST_ENTRY Queue,
    _Inout_ ULONG* Count,
    _In_reads_bytes_(Length) const UCHAR* Data,
    _In_ SIZE_T Length
    )
{
    if (Length < WINTAP_FRAME_MINIMUM || Length > WINTAP_FRAME_MAXIMUM) {
        return STATUS_INVALID_BUFFER_SIZE;
    }

    SIZE_T allocationSize = FIELD_OFFSET(WINTAP_FRAME, Data) + Length;
    PWINTAP_FRAME frame = static_cast<PWINTAP_FRAME>(
        ExAllocatePool2(POOL_FLAG_NON_PAGED, allocationSize, 'paTW'));
    if (frame == nullptr) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }

    frame->Length = Length;
    RtlCopyMemory(frame->Data, Data, Length);

    WdfSpinLockAcquire(Context->FrameLock);
    if (Context->Closing || Context->Removed || Context->Suspended ||
        *Count >= Context->FrameLimit) {
        WdfSpinLockRelease(Context->FrameLock);
        ExFreePoolWithTag(frame, 'paTW');
        return (Context->Closing || Context->Removed || Context->Suspended)
            ? STATUS_DEVICE_NOT_READY
            : STATUS_DEVICE_BUSY;
    }

    InsertTailList(Queue, &frame->ListEntry);
    ++*Count;
    WdfSpinLockRelease(Context->FrameLock);
    return STATUS_SUCCESS;
}

static PWINTAP_FRAME
WintapDequeueFrame(
    _In_ PWINTAP_DEVICE_CONTEXT Context,
    _In_ PLIST_ENTRY Queue,
    _Inout_ ULONG* Count
    )
{
    PWINTAP_FRAME frame = nullptr;

    WdfSpinLockAcquire(Context->FrameLock);
    if (!IsListEmpty(Queue)) {
        PLIST_ENTRY entry = RemoveHeadList(Queue);
        frame = CONTAINING_RECORD(entry, WINTAP_FRAME, ListEntry);
        --*Count;
    }
    WdfSpinLockRelease(Context->FrameLock);
    return frame;
}

static VOID
WintapCompletePendingReads(
    _In_ PWINTAP_DEVICE_CONTEXT Context
    )
{
    for (;;) {
        PWINTAP_FRAME frame = WintapDequeueFrame(
            Context,
            &Context->FromStackQueue,
            &Context->FromStackCount);
        if (frame == nullptr) {
            return;
        }

        WDFREQUEST request = nullptr;
        NTSTATUS status = WdfIoQueueRetrieveNextRequest(Context->ReadQueue, &request);
        if (!NT_SUCCESS(status)) {
            WdfSpinLockAcquire(Context->FrameLock);
            InsertHeadList(&Context->FromStackQueue, &frame->ListEntry);
            ++Context->FromStackCount;
            WdfSpinLockRelease(Context->FrameLock);
            return;
        }
        WintapReleasePendingRequest(Context, request);

        PVOID outputBuffer = nullptr;
        size_t outputLength = 0;
        status = WdfRequestRetrieveOutputBuffer(
            request,
            frame->Length,
            &outputBuffer,
            &outputLength);
        if (NT_SUCCESS(status)) {
            RtlCopyMemory(outputBuffer, frame->Data, frame->Length);
            WdfRequestCompleteWithInformation(request, STATUS_SUCCESS, frame->Length);
            ExFreePoolWithTag(frame, 'paTW');
        } else {
            WdfRequestComplete(request, status);
            WdfSpinLockAcquire(Context->FrameLock);
            InsertHeadList(&Context->FromStackQueue, &frame->ListEntry);
            ++Context->FromStackCount;
            WdfSpinLockRelease(Context->FrameLock);
        }
    }
}

static NTSTATUS
WintapProcessWriteRequest(
    _In_ PWINTAP_DEVICE_CONTEXT Context,
    _In_ WDFREQUEST Request,
    _Out_ size_t* BytesAccepted
    )
{
    BOOLEAN closing;

    WdfSpinLockAcquire(Context->FrameLock);
    closing = Context->Closing || Context->Removed || Context->Suspended;
    WdfSpinLockRelease(Context->FrameLock);
    if (closing) {
        return STATUS_DEVICE_NOT_READY;
    }

    PVOID inputBuffer = nullptr;
    size_t inputLength = 0;
    NTSTATUS status = WdfRequestRetrieveInputBuffer(
        Request,
        WINTAP_FRAME_MINIMUM,
        &inputBuffer,
        &inputLength);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    if (inputLength > WINTAP_FRAME_MAXIMUM) {
        return STATUS_INVALID_BUFFER_SIZE;
    }

    status = WintapQueueFrame(
        Context,
        &Context->ToStackQueue,
        &Context->ToStackCount,
        static_cast<const UCHAR*>(inputBuffer),
        inputLength);
    if (NT_SUCCESS(status)) {
        *BytesAccepted = inputLength;
    }
    return status;
}

static VOID
WintapDrainPendingWrites(
    _In_ PWINTAP_DEVICE_CONTEXT Context
    )
{
    for (;;) {
        WDFREQUEST request = nullptr;
        NTSTATUS status = WdfIoQueueRetrieveNextRequest(
            Context->WriteQueue,
            &request);
        if (!NT_SUCCESS(status)) {
            return;
        }
        WintapReleasePendingRequest(Context, request);

        size_t bytesAccepted = 0;
        status = WintapProcessWriteRequest(Context, request, &bytesAccepted);
        if (status == STATUS_DEVICE_BUSY) {
            status = WintapMarkPendingRequest(Context, request, FALSE);
            if (NT_SUCCESS(status)) {
                status = WdfRequestForwardToIoQueue(request, Context->WriteQueue);
                if (!NT_SUCCESS(status)) {
                    WintapReleasePendingRequest(Context, request);
                }
            }
            if (!NT_SUCCESS(status)) {
                WdfRequestComplete(request, status);
            }
            return;
        }

        if (NT_SUCCESS(status)) {
            WdfRequestCompleteWithInformation(
                request,
                STATUS_SUCCESS,
                bytesAccepted);
            WintapCompletePendingReads(Context);
        } else {
            WdfRequestComplete(request, status);
        }
    }
}

static VOID
WintapScheduleWriteDrain(
    _In_ PWINTAP_DEVICE_CONTEXT Context
    )
{
    BOOLEAN enqueue = FALSE;
    WdfSpinLockAcquire(Context->FrameLock);
    if (!Context->WriteDrainQueued && !Context->Closing) {
        Context->WriteDrainQueued = TRUE;
        KeClearEvent(&Context->WriteDrainIdle);
        enqueue = TRUE;
    }
    WdfSpinLockRelease(Context->FrameLock);

    if (enqueue) {
        WdfWorkItemEnqueue(Context->WriteDrainWorkItem);
    }
}

static VOID
WintapScheduleReadCompletion(
    _In_ PWINTAP_DEVICE_CONTEXT Context
    )
{
    BOOLEAN enqueue = FALSE;
    WdfSpinLockAcquire(Context->FrameLock);
    if (!Context->ReadCompletionQueued &&
        !Context->Closing &&
        !Context->Removed &&
        !Context->Suspended) {
        Context->ReadCompletionQueued = TRUE;
        KeClearEvent(&Context->ReadCompletionIdle);
        enqueue = TRUE;
    }
    WdfSpinLockRelease(Context->FrameLock);

    if (enqueue) {
        WdfWorkItemEnqueue(Context->ReadCompletionWorkItem);
    }
}

static NTSTATUS
WintapCreateControlQueue(
    _In_ WDFDEVICE Device,
    _Out_ WDFQUEUE* Queue
    )
{
    WDF_IO_QUEUE_CONFIG config;
    WDF_IO_QUEUE_CONFIG_INIT_DEFAULT_QUEUE(&config, WdfIoQueueDispatchParallel);
    config.EvtIoRead = WintapEvtIoRead;
    config.EvtIoWrite = WintapEvtIoWrite;
    config.EvtIoStop = nullptr;

    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    return WdfIoQueueCreate(Device, &config, &attributes, Queue);
}

NTSTATUS
WintapCreateControlDevice(
    _In_ WDFDRIVER Driver
    )
{
    DECLARE_CONST_UNICODE_STRING(sddl, L"D:P(A;;GA;;;BA)");
    PWDFDEVICE_INIT deviceInit = WdfControlDeviceInitAllocate(Driver, &sddl);
    if (deviceInit == nullptr) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }

    DECLARE_CONST_UNICODE_STRING(deviceName, L"\\Device\\WinTapNetAdapterCx");
    DECLARE_CONST_UNICODE_STRING(symbolicLink, L"\\DosDevices\\WinTapNetAdapterCx");
    NTSTATUS status = WdfDeviceInitAssignName(deviceInit, &deviceName);
    if (!NT_SUCCESS(status)) {
        WdfDeviceInitFree(deviceInit);
        return status;
    }

    WDF_FILEOBJECT_CONFIG fileConfig;
    WDF_FILEOBJECT_CONFIG_INIT(
        &fileConfig,
        WintapEvtFileCreate,
        WintapEvtFileClose,
        WintapEvtFileCleanup);
    WdfDeviceInitSetFileObjectConfig(deviceInit, &fileConfig, WDF_NO_OBJECT_ATTRIBUTES);
    WdfDeviceInitSetExclusive(deviceInit, TRUE);

    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, WINTAP_DEVICE_CONTEXT);

    WDFDEVICE device = nullptr;
    status = WdfDeviceCreate(&deviceInit, &attributes, &device);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    PWINTAP_DEVICE_CONTEXT context = WintapGetDeviceContext(device);
    context->FrameLimit = WINTAP_FRAME_LIMIT;
    context->PendingReadLimit = WINTAP_PENDING_READ_LIMIT;
    context->PendingWriteLimit = WINTAP_PENDING_WRITE_LIMIT;
    context->PendingReadCount = 0;
    context->PendingWriteCount = 0;
    context->Closing = FALSE;
    context->Removed = FALSE;
    context->Suspended = FALSE;
    context->WriteDrainQueued = FALSE;
    context->WriteDrainRunning = FALSE;
    context->ReadCompletionQueued = FALSE;
    context->ReadCompletionRunning = FALSE;
    context->ActiveCallbacks = 0;
    KeInitializeEvent(&context->CallbackIdle, NotificationEvent, TRUE);
    KeInitializeEvent(&context->WriteDrainIdle, NotificationEvent, TRUE);
    KeInitializeEvent(&context->ReadCompletionIdle, NotificationEvent, TRUE);
    InitializeListHead(&context->ToStackQueue);
    InitializeListHead(&context->FromStackQueue);

    WDF_OBJECT_ATTRIBUTES lockAttributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&lockAttributes);
    status = WdfSpinLockCreate(&lockAttributes, &context->FrameLock);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    status = WintapCreateControlQueue(device, &context->DefaultQueue);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    WDF_IO_QUEUE_CONFIG readConfig;
    WDF_IO_QUEUE_CONFIG_INIT(&readConfig, WdfIoQueueDispatchManual);
    readConfig.EvtIoStop = WintapEvtIoStop;
    WDF_OBJECT_ATTRIBUTES readAttributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&readAttributes);
    status = WdfIoQueueCreate(device, &readConfig, &readAttributes, &context->ReadQueue);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    WDF_IO_QUEUE_CONFIG writeConfig;
    WDF_IO_QUEUE_CONFIG_INIT(&writeConfig, WdfIoQueueDispatchManual);
    writeConfig.EvtIoStop = WintapEvtIoStop;
    WDF_OBJECT_ATTRIBUTES writeAttributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&writeAttributes);
    status = WdfIoQueueCreate(
        device,
        &writeConfig,
        &writeAttributes,
        &context->WriteQueue);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    WDF_WORKITEM_CONFIG workItemConfig;
    WDF_WORKITEM_CONFIG_INIT(
        &workItemConfig,
        WintapEvtWriteDrainWorkItem);
    WDF_OBJECT_ATTRIBUTES workItemAttributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&workItemAttributes);
    workItemAttributes.ParentObject = device;
    status = WdfWorkItemCreate(
        &workItemConfig,
        &workItemAttributes,
        &context->WriteDrainWorkItem);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    WDF_WORKITEM_CONFIG_INIT(
        &workItemConfig,
        WintapEvtReadCompletionWorkItem);
    status = WdfWorkItemCreate(
        &workItemConfig,
        &workItemAttributes,
        &context->ReadCompletionWorkItem);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    status = WdfDeviceCreateSymbolicLink(device, &symbolicLink);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    WdfControlFinishInitializing(device);
    KIRQL oldIrql;
    KeAcquireSpinLock(&g_ControlContextLock, &oldIrql);
    g_ControlDevice = device;
    g_ControlContext = context;
    KeReleaseSpinLock(&g_ControlContextLock, oldIrql);
    return STATUS_SUCCESS;
}

static NTSTATUS
WintapCreateAdapter(
    _In_ WDFDEVICE Device
    )
{
    NET_ADAPTER_DATAPATH_CALLBACKS callbacks;
    NET_ADAPTER_DATAPATH_CALLBACKS_INIT(
        &callbacks,
        WintapEvtCreateTxQueue,
        WintapEvtCreateRxQueue);

    NETADAPTER_INIT* adapterInit = NetAdapterInitAllocate(Device);
    if (adapterInit == nullptr) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }

    NetAdapterInitSetDatapathCallbacks(adapterInit, &callbacks);

    NETADAPTER adapter = nullptr;
    NTSTATUS status = NetAdapterCreate(
        adapterInit,
        WDF_NO_OBJECT_ATTRIBUTES,
        &adapter);

    if (!NT_SUCCESS(status)) {
        NetAdapterInitFree(adapterInit);
        return status;
    }

    NET_ADAPTER_LINK_LAYER_CAPABILITIES capabilities;
    NET_ADAPTER_LINK_LAYER_CAPABILITIES_INIT(
        &capabilities,
        1000000000ULL,
        1000000000ULL);
    NetAdapterSetLinkLayerCapabilities(adapter, &capabilities);
    NetAdapterSetLinkLayerMtuSize(adapter, WINTAP_FRAME_MAXIMUM - WINTAP_FRAME_MINIMUM);

    status = NetAdapterStart(adapter);
    if (!NT_SUCCESS(status)) {
        WdfObjectDelete(adapter);
    } else {
        KIRQL oldIrql;
        KeAcquireSpinLock(&g_ControlContextLock, &oldIrql);
        g_Adapter = adapter;
        KeReleaseSpinLock(&g_ControlContextLock, oldIrql);
    }

    return status;
}

static BOOLEAN
WintapValidateFragment(
    _In_ const NET_FRAGMENT* Fragment,
    _In_ const NET_FRAGMENT_VIRTUAL_ADDRESS* Address,
    _Inout_ SIZE_T* TotalLength
    )
{
    if (Fragment == nullptr ||
        Address == nullptr ||
        Address->VirtualAddress == nullptr) {
        return FALSE;
    }

    SIZE_T offset = Fragment->Offset;
    SIZE_T capacity = Fragment->Capacity;
    SIZE_T validLength = Fragment->ValidLength;
    if (offset > capacity ||
        validLength > capacity - offset ||
        validLength > WINTAP_FRAME_MAXIMUM ||
        *TotalLength > WINTAP_FRAME_MAXIMUM - validLength) {
        return FALSE;
    }

    *TotalLength += validLength;
    return TRUE;
}

_Use_decl_annotations_
NTSTATUS
DriverEntry(
    PDRIVER_OBJECT DriverObject,
    PUNICODE_STRING RegistryPath
    )
{
    WDF_DRIVER_CONFIG config;
    WDF_DRIVER_CONFIG_INIT(&config, WintapEvtDeviceAdd);
    KeInitializeSpinLock(&g_ControlContextLock);

    return WdfDriverCreate(
        DriverObject,
        RegistryPath,
        WDF_NO_OBJECT_ATTRIBUTES,
        &config,
        WDF_NO_HANDLE);
}

_Use_decl_annotations_
NTSTATUS
WintapEvtDeviceAdd(
    WDFDRIVER Driver,
    PWDFDEVICE_INIT DeviceInit
    )
{
    UNREFERENCED_PARAMETER(Driver);

    NTSTATUS status = NetDeviceInitConfig(DeviceInit);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    WDF_PNPPOWER_EVENT_CALLBACKS callbacks;
    WDF_PNPPOWER_EVENT_CALLBACKS_INIT(&callbacks);
    callbacks.EvtDevicePrepareHardware = WintapEvtPrepareHardware;
    callbacks.EvtDeviceReleaseHardware = WintapEvtReleaseHardware;
    callbacks.EvtDeviceD0Entry = WintapEvtDeviceD0Entry;
    callbacks.EvtDeviceD0Exit = WintapEvtDeviceD0Exit;
    WdfDeviceInitSetPnpPowerEventCallbacks(DeviceInit, &callbacks);

    WDFDEVICE device = nullptr;
    status = WdfDeviceCreate(&DeviceInit, WDF_NO_OBJECT_ATTRIBUTES, &device);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    status = WintapCreateControlDevice(WdfDeviceGetDriver(device));
    if (!NT_SUCCESS(status)) {
        return status;
    }

    status = WintapCreateAdapter(device);
    if (!NT_SUCCESS(status)) {
        WDFDEVICE controlDevice;
        KIRQL oldIrql;
        KeAcquireSpinLock(&g_ControlContextLock, &oldIrql);
        controlDevice = g_ControlDevice;
        g_Adapter = nullptr;
        g_ControlDevice = nullptr;
        g_ControlContext = nullptr;
        KeReleaseSpinLock(&g_ControlContextLock, oldIrql);
        if (controlDevice != nullptr) {
            WdfObjectDelete(controlDevice);
        }
    }
    return status;
}

_Use_decl_annotations_
NTSTATUS
WintapEvtPrepareHardware(
    WDFDEVICE Device,
    WDFCMRESLIST ResourcesRaw,
    WDFCMRESLIST ResourcesTranslated
    )
{
    UNREFERENCED_PARAMETER(Device);
    UNREFERENCED_PARAMETER(ResourcesRaw);
    UNREFERENCED_PARAMETER(ResourcesTranslated);

    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS
WintapEvtReleaseHardware(
    WDFDEVICE Device,
    WDFCMRESLIST ResourcesTranslated
    )
{
    UNREFERENCED_PARAMETER(Device);
    UNREFERENCED_PARAMETER(ResourcesTranslated);

    PWINTAP_DEVICE_CONTEXT context;
    NETADAPTER adapter;
    WDFDEVICE controlDevice;
    KIRQL oldIrql;

    KeAcquireSpinLock(&g_ControlContextLock, &oldIrql);
    context = g_ControlContext;
    adapter = g_Adapter;
    controlDevice = g_ControlDevice;
    g_ControlContext = nullptr;
    g_ControlDevice = nullptr;
    g_Adapter = nullptr;
    if (context != nullptr) {
        WdfSpinLockAcquire(context->FrameLock);
        context->Removed = TRUE;
        context->Closing = TRUE;
        context->Suspended = TRUE;
        WdfSpinLockRelease(context->FrameLock);
    }
    KeReleaseSpinLock(&g_ControlContextLock, oldIrql);

    if (context == nullptr) {
        if (controlDevice != nullptr) {
            WdfObjectDelete(controlDevice);
        }
        return STATUS_SUCCESS;
    }

    if (adapter != nullptr) {
        NetAdapterStop(adapter);
    }
    KeWaitForSingleObject(
        &context->CallbackIdle,
        Executive,
        KernelMode,
        FALSE,
        nullptr);
    if (context->ReadQueue != nullptr) {
        WdfIoQueuePurgeSynchronously(context->ReadQueue);
        WdfIoQueuePurgeSynchronously(context->WriteQueue);
    }
    WdfSpinLockAcquire(context->FrameLock);
    context->PendingReadCount = 0;
    context->PendingWriteCount = 0;
    WdfSpinLockRelease(context->FrameLock);
    WintapWaitForWriteDrain(context);
    WintapWaitForReadCompletion(context);
    WintapFreeFrames(context);
    if (controlDevice != nullptr) {
        WdfObjectDelete(controlDevice);
    }
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS
WintapEvtDeviceD0Entry(
    WDFDEVICE Device,
    WDF_POWER_DEVICE_STATE PreviousState
    )
{
    UNREFERENCED_PARAMETER(Device);
    UNREFERENCED_PARAMETER(PreviousState);

    PWINTAP_DEVICE_CONTEXT context = WintapPeekControlContext();
    if (context != nullptr) {
        WdfSpinLockAcquire(context->FrameLock);
        if (!context->Removed && !context->Closing) {
            context->Suspended = FALSE;
        }
        WdfSpinLockRelease(context->FrameLock);
        WintapScheduleWriteDrain(context);
        WintapScheduleReadCompletion(context);
    }
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS
WintapEvtDeviceD0Exit(
    WDFDEVICE Device,
    WDF_POWER_DEVICE_STATE TargetState
    )
{
    UNREFERENCED_PARAMETER(Device);
    UNREFERENCED_PARAMETER(TargetState);

    PWINTAP_DEVICE_CONTEXT context = WintapPeekControlContext();
    if (context == nullptr) {
        return STATUS_SUCCESS;
    }

    WdfSpinLockAcquire(context->FrameLock);
    context->Suspended = TRUE;
    WdfSpinLockRelease(context->FrameLock);
    KeWaitForSingleObject(
        &context->CallbackIdle,
        Executive,
        KernelMode,
        FALSE,
        nullptr);
    if (context->ReadQueue != nullptr) {
        WdfIoQueuePurgeSynchronously(context->ReadQueue);
        WdfIoQueuePurgeSynchronously(context->WriteQueue);
    }
    WdfSpinLockAcquire(context->FrameLock);
    context->PendingReadCount = 0;
    context->PendingWriteCount = 0;
    WdfSpinLockRelease(context->FrameLock);
    WintapWaitForWriteDrain(context);
    WintapWaitForReadCompletion(context);
    WintapFreeFrames(context);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
VOID
WintapEvtFileCreate(
    WDFDEVICE Device,
    WDFREQUEST Request,
    WDFFILEOBJECT FileObject
    )
{
    UNREFERENCED_PARAMETER(Device);
    PWINTAP_DEVICE_CONTEXT context = WintapGetDeviceContext(Device);
    WdfSpinLockAcquire(context->FrameLock);
    BOOLEAN removed = context->Removed;
    WdfSpinLockRelease(context->FrameLock);
    if (removed) {
        WdfRequestComplete(Request, STATUS_DEVICE_REMOVED);
        return;
    }
    KIRQL oldIrql;
    BOOLEAN busy = FALSE;
    KeAcquireSpinLock(&g_ControlContextLock, &oldIrql);
    if (g_ControlContext == nullptr) {
        g_ControlContext = context;
    } else if (g_ControlContext != context) {
        busy = TRUE;
    }
    KeReleaseSpinLock(&g_ControlContextLock, oldIrql);
    UNREFERENCED_PARAMETER(FileObject);
    WdfRequestComplete(
        Request,
        busy ? STATUS_DEVICE_BUSY : STATUS_SUCCESS);
}

_Use_decl_annotations_
VOID
WintapEvtFileCleanup(
    WDFFILEOBJECT FileObject
    )
{
    WDFDEVICE device = WdfFileObjectGetDevice(FileObject);
    PWINTAP_DEVICE_CONTEXT context = WintapGetDeviceContext(device);
    WintapClearControlContext(context);
    WdfSpinLockAcquire(context->FrameLock);
    context->Closing = TRUE;
    WdfSpinLockRelease(context->FrameLock);
    WdfIoQueuePurgeSynchronously(context->ReadQueue);
    WdfIoQueuePurgeSynchronously(context->WriteQueue);
    WdfSpinLockAcquire(context->FrameLock);
    context->PendingReadCount = 0;
    context->PendingWriteCount = 0;
    WdfSpinLockRelease(context->FrameLock);
    KeWaitForSingleObject(
        &context->CallbackIdle,
        Executive,
        KernelMode,
        FALSE,
        nullptr);
    WintapWaitForWriteDrain(context);
    WintapWaitForReadCompletion(context);
    WintapFreeFrames(context);
    WdfSpinLockAcquire(context->FrameLock);
    context->Closing = context->Removed;
    WdfSpinLockRelease(context->FrameLock);
}

_Use_decl_annotations_
VOID
WintapEvtFileClose(
    WDFFILEOBJECT FileObject
    )
{
    UNREFERENCED_PARAMETER(FileObject);
}

_Use_decl_annotations_
VOID
WintapEvtWriteDrainWorkItem(
    WDFWORKITEM WorkItem
    )
{
    WDFDEVICE device = static_cast<WDFDEVICE>(
        WdfWorkItemGetParentObject(WorkItem));
    PWINTAP_DEVICE_CONTEXT context = WintapGetDeviceContext(device);
    WdfSpinLockAcquire(context->FrameLock);
    context->WriteDrainQueued = FALSE;
    context->WriteDrainRunning = TRUE;
    KeClearEvent(&context->WriteDrainIdle);
    BOOLEAN closing = context->Closing || context->Removed || context->Suspended;
    WdfSpinLockRelease(context->FrameLock);
    if (!closing) {
        WintapDrainPendingWrites(context);
    }
    WdfSpinLockAcquire(context->FrameLock);
    context->WriteDrainRunning = FALSE;
    KeSetEvent(&context->WriteDrainIdle, IO_NO_INCREMENT, FALSE);
    WdfSpinLockRelease(context->FrameLock);
}

_Use_decl_annotations_
VOID
WintapEvtReadCompletionWorkItem(
    WDFWORKITEM WorkItem
    )
{
    WDFDEVICE device = static_cast<WDFDEVICE>(
        WdfWorkItemGetParentObject(WorkItem));
    PWINTAP_DEVICE_CONTEXT context = WintapGetDeviceContext(device);
    WdfSpinLockAcquire(context->FrameLock);
    context->ReadCompletionQueued = FALSE;
    context->ReadCompletionRunning = TRUE;
    KeClearEvent(&context->ReadCompletionIdle);
    WdfSpinLockRelease(context->FrameLock);

    WintapCompletePendingReads(context);

    WdfSpinLockAcquire(context->FrameLock);
    context->ReadCompletionRunning = FALSE;
    KeSetEvent(&context->ReadCompletionIdle, IO_NO_INCREMENT, FALSE);
    WdfSpinLockRelease(context->FrameLock);
}

_Use_decl_annotations_
VOID
WintapEvtIoRead(
    WDFQUEUE Queue,
    WDFREQUEST Request,
    size_t Length
    )
{
    UNREFERENCED_PARAMETER(Length);
    WDFDEVICE device = WdfIoQueueGetDevice(Queue);
    PWINTAP_DEVICE_CONTEXT context = WintapGetDeviceContext(device);
    WdfSpinLockAcquire(context->FrameLock);
    BOOLEAN removed = context->Removed;
    BOOLEAN closing = context->Closing || removed || context->Suspended;
    WdfSpinLockRelease(context->FrameLock);
    if (closing) {
        WdfRequestComplete(
            Request,
            removed ? STATUS_DEVICE_REMOVED : STATUS_DEVICE_NOT_READY);
        return;
    }

    PWINTAP_FRAME frame = WintapDequeueFrame(
        context,
        &context->FromStackQueue,
        &context->FromStackCount);
    if (frame == nullptr) {
        NTSTATUS markStatus = WintapMarkPendingRequest(
            context,
            Request,
            TRUE);
        if (!NT_SUCCESS(markStatus)) {
            WdfRequestComplete(Request, markStatus);
            return;
        }
        NTSTATUS status = WdfRequestForwardToIoQueue(Request, context->ReadQueue);
        if (!NT_SUCCESS(status)) {
            WintapReleasePendingRequest(context, Request);
            WdfRequestComplete(Request, status);
        }
        return;
    }

    PVOID outputBuffer = nullptr;
    size_t outputLength = 0;
    NTSTATUS status = WdfRequestRetrieveOutputBuffer(
        Request,
        frame->Length,
        &outputBuffer,
        &outputLength);
    if (NT_SUCCESS(status)) {
        RtlCopyMemory(outputBuffer, frame->Data, frame->Length);
        WdfRequestCompleteWithInformation(Request, STATUS_SUCCESS, frame->Length);
    } else {
        WdfRequestComplete(Request, status);
    }
    ExFreePoolWithTag(frame, 'paTW');
    WintapDrainPendingWrites(context);
}

_Use_decl_annotations_
VOID
WintapEvtIoWrite(
    WDFQUEUE Queue,
    WDFREQUEST Request,
    size_t Length
    )
{
    UNREFERENCED_PARAMETER(Length);
    WDFDEVICE device = WdfIoQueueGetDevice(Queue);
    PWINTAP_DEVICE_CONTEXT context = WintapGetDeviceContext(device);
    WdfSpinLockAcquire(context->FrameLock);
    BOOLEAN removed = context->Removed;
    BOOLEAN closing = context->Closing || removed || context->Suspended;
    WdfSpinLockRelease(context->FrameLock);
    if (closing) {
        WdfRequestComplete(
            Request,
            removed ? STATUS_DEVICE_REMOVED : STATUS_DEVICE_NOT_READY);
        return;
    }

    size_t bytesAccepted = 0;
    NTSTATUS status = WintapProcessWriteRequest(
        context,
        Request,
        &bytesAccepted);
    if (status == STATUS_DEVICE_BUSY) {
        status = WintapMarkPendingRequest(context, Request, FALSE);
        if (!NT_SUCCESS(status)) {
            WdfRequestComplete(Request, status);
            return;
        }
        status = WdfRequestForwardToIoQueue(Request, context->WriteQueue);
        if (!NT_SUCCESS(status)) {
            WintapReleasePendingRequest(context, Request);
            WdfRequestComplete(Request, status);
        }
        return;
    }

    if (NT_SUCCESS(status)) {
        WdfRequestCompleteWithInformation(
            Request,
            STATUS_SUCCESS,
            bytesAccepted);
    } else {
        WdfRequestComplete(Request, status);
    }
}

_Use_decl_annotations_
VOID
WintapEvtIoStop(
    WDFQUEUE Queue,
    WDFREQUEST Request,
    ULONG ActionFlags
    )
{
    UNREFERENCED_PARAMETER(ActionFlags);
    WDFDEVICE device = WdfIoQueueGetDevice(Queue);
    WintapReleasePendingRequest(
        WintapGetDeviceContext(device),
        Request);
}

_Use_decl_annotations_
NTSTATUS
WintapEvtCreateTxQueue(
    NETADAPTER Adapter,
    NETTXQUEUE_INIT* QueueInit
    )
{
    UNREFERENCED_PARAMETER(Adapter);

    NET_PACKET_QUEUE_CONFIG config;
    NET_PACKET_QUEUE_CONFIG_INIT(
        &config,
        WintapEvtPacketQueueAdvance,
        WintapEvtPacketQueueSetNotificationEnabled,
        WintapEvtPacketQueueCancel);
    config.EvtStart = WintapEvtPacketQueueStart;
    config.EvtStop = WintapEvtPacketQueueStop;

    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, WINTAP_QUEUE_CONTEXT);
    NETPACKETQUEUE queue = nullptr;
    NTSTATUS status = NetTxQueueCreate(
        QueueInit,
        &attributes,
        &config,
        &queue);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    PWINTAP_QUEUE_CONTEXT context = WintapGetQueueContext(queue);
    context->IsTransmit = TRUE;
    context->Started = FALSE;
    context->Rings = NetTxQueueGetRingCollection(queue);
    NET_EXTENSION_QUERY query;
    NET_EXTENSION_QUERY_INIT(
        &query,
        NET_FRAGMENT_EXTENSION_VIRTUAL_ADDRESS_NAME,
        NET_FRAGMENT_EXTENSION_VIRTUAL_ADDRESS_VERSION_1,
        NetExtensionTypeFragment);
    NetTxQueueGetExtension(queue, &query, &context->FragmentVirtualAddress);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS
WintapEvtCreateRxQueue(
    NETADAPTER Adapter,
    NETRXQUEUE_INIT* QueueInit
    )
{
    UNREFERENCED_PARAMETER(Adapter);

    NET_PACKET_QUEUE_CONFIG config;
    NET_PACKET_QUEUE_CONFIG_INIT(
        &config,
        WintapEvtPacketQueueAdvance,
        WintapEvtPacketQueueSetNotificationEnabled,
        WintapEvtPacketQueueCancel);
    config.EvtStart = WintapEvtPacketQueueStart;
    config.EvtStop = WintapEvtPacketQueueStop;

    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, WINTAP_QUEUE_CONTEXT);
    NETPACKETQUEUE queue = nullptr;
    NTSTATUS status = NetRxQueueCreate(
        QueueInit,
        &attributes,
        &config,
        &queue);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    PWINTAP_QUEUE_CONTEXT context = WintapGetQueueContext(queue);
    context->IsTransmit = FALSE;
    context->Started = FALSE;
    context->Rings = NetRxQueueGetRingCollection(queue);
    NET_EXTENSION_QUERY query;
    NET_EXTENSION_QUERY_INIT(
        &query,
        NET_FRAGMENT_EXTENSION_VIRTUAL_ADDRESS_NAME,
        NET_FRAGMENT_EXTENSION_VIRTUAL_ADDRESS_VERSION_1,
        NetExtensionTypeFragment);
    NetRxQueueGetExtension(queue, &query, &context->FragmentVirtualAddress);
    return STATUS_SUCCESS;
}

static VOID
WintapCaptureTransmitPackets(
    _In_ PWINTAP_QUEUE_CONTEXT QueueContext,
    _In_ PWINTAP_DEVICE_CONTEXT ControlContext
    )
{
    NET_RING* packetRing = NetRingCollectionGetPacketRing(QueueContext->Rings);
    NET_RING* fragmentRing = NetRingCollectionGetFragmentRing(QueueContext->Rings);
    if (packetRing == nullptr || fragmentRing == nullptr) {
        return;
    }

    while (packetRing->BeginIndex != packetRing->EndIndex) {
        UINT32 packetIndex = packetRing->BeginIndex;
        NET_PACKET* packet = NetRingGetPacketAtIndex(packetRing, packetIndex);
        if (packet == nullptr) {
            return;
        }

        SIZE_T length = 0;
        UINT32 fragmentIndex = packet->FragmentIndex;
        BOOLEAN valid = packet->FragmentCount != 0 &&
            packet->FragmentCount <= WINTAP_FRAME_MAXIMUM;
        for (UINT16 i = 0; valid && i < packet->FragmentCount; ++i) {
            NET_FRAGMENT* fragment = NetRingGetFragmentAtIndex(
                fragmentRing,
                fragmentIndex);
            NET_FRAGMENT_VIRTUAL_ADDRESS* address =
                NetExtensionGetFragmentVirtualAddress(
                    &QueueContext->FragmentVirtualAddress,
                    fragmentIndex);
            valid = WintapValidateFragment(fragment, address, &length);
            fragmentIndex = NetRingIncrementIndex(fragmentRing, fragmentIndex);
        }

        if (valid &&
            length >= WINTAP_FRAME_MINIMUM &&
            length <= WINTAP_FRAME_MAXIMUM) {
            SIZE_T allocationSize = FIELD_OFFSET(WINTAP_FRAME, Data) + length;
            PWINTAP_FRAME frame = static_cast<PWINTAP_FRAME>(
                ExAllocatePool2(POOL_FLAG_NON_PAGED, allocationSize, 'paTW'));
            if (frame != nullptr) {
                SIZE_T copied = 0;
                fragmentIndex = packet->FragmentIndex;
                valid = TRUE;
                for (UINT16 i = 0; valid && i < packet->FragmentCount; ++i) {
                    NET_FRAGMENT* fragment = NetRingGetFragmentAtIndex(
                        fragmentRing,
                        fragmentIndex);
                    NET_FRAGMENT_VIRTUAL_ADDRESS* address =
                        NetExtensionGetFragmentVirtualAddress(
                            &QueueContext->FragmentVirtualAddress,
                            fragmentIndex);
                    SIZE_T before = copied;
                    valid = WintapValidateFragment(
                        fragment,
                        address,
                        &copied);
                    SIZE_T fragmentLength = copied - before;
                    if (!valid) {
                        break;
                    }
                    RtlCopyMemory(
                        frame->Data + before,
                        static_cast<UCHAR*>(address->VirtualAddress) +
                            fragment->Offset,
                        fragmentLength);
                    fragmentIndex = NetRingIncrementIndex(
                        fragmentRing,
                        fragmentIndex);
                }
                if (!valid || copied != length) {
                    ExFreePoolWithTag(frame, 'paTW');
                    frame = nullptr;
                }

                if (frame != nullptr) {
                    frame->Length = copied;
                    WdfSpinLockAcquire(ControlContext->FrameLock);
                    if (!ControlContext->Closing &&
                        !ControlContext->Removed &&
                        !ControlContext->Suspended &&
                        ControlContext->FromStackCount <
                            ControlContext->FrameLimit) {
                        InsertTailList(
                            &ControlContext->FromStackQueue,
                            &frame->ListEntry);
                        ++ControlContext->FromStackCount;
                        frame = nullptr;
                    }
                    WdfSpinLockRelease(ControlContext->FrameLock);
                }
                if (frame != nullptr) {
                    ExFreePoolWithTag(frame, 'paTW');
                }
            }
        }

        packetRing->BeginIndex = NetRingIncrementIndex(
            packetRing,
            packetRing->BeginIndex);
        fragmentRing->BeginIndex = NetRingAdvanceIndex(
            fragmentRing,
            fragmentRing->BeginIndex,
            packet->FragmentCount);
    }

    WintapScheduleReadCompletion(ControlContext);
}

static VOID
WintapInjectReceiveFrames(
    _In_ PWINTAP_QUEUE_CONTEXT QueueContext,
    _In_ PWINTAP_DEVICE_CONTEXT ControlContext
    )
{
    NET_RING* packetRing = NetRingCollectionGetPacketRing(QueueContext->Rings);
    NET_RING* fragmentRing = NetRingCollectionGetFragmentRing(QueueContext->Rings);
    if (packetRing == nullptr || fragmentRing == nullptr) {
        return;
    }
    BOOLEAN releasedFrame = FALSE;

    while (packetRing->BeginIndex != packetRing->EndIndex &&
           fragmentRing->BeginIndex != fragmentRing->EndIndex) {
        PWINTAP_FRAME frame = WintapDequeueFrame(
            ControlContext,
            &ControlContext->ToStackQueue,
            &ControlContext->ToStackCount);
        if (frame == nullptr) {
            if (releasedFrame) {
                WintapScheduleWriteDrain(ControlContext);
            }
            return;
        }
        releasedFrame = TRUE;

        UINT32 packetIndex = packetRing->BeginIndex;
        UINT32 fragmentIndex = fragmentRing->BeginIndex;
        NET_PACKET* packet = NetRingGetPacketAtIndex(packetRing, packetIndex);
        NET_FRAGMENT* fragment = NetRingGetFragmentAtIndex(
            fragmentRing,
            fragmentIndex);
        NET_FRAGMENT_VIRTUAL_ADDRESS* address =
            NetExtensionGetFragmentVirtualAddress(
                &QueueContext->FragmentVirtualAddress,
                fragmentIndex);
        SIZE_T totalLength = 0;
        if (!WintapValidateFragment(fragment, address, &totalLength) ||
            fragment->Offset > fragment->Capacity ||
            frame->Length > fragment->Capacity - fragment->Offset) {
            ExFreePoolWithTag(frame, 'paTW');
            continue;
        }

        RtlCopyMemory(
            static_cast<UCHAR*>(address->VirtualAddress) + fragment->Offset,
            frame->Data,
            frame->Length);
        fragment->ValidLength = frame->Length;
        packet->FragmentIndex = fragmentIndex;
        packet->FragmentCount = 1;
        packet->Layout.Layer2Type = NetPacketLayer2TypeEthernet;
        packet->Layout.Layer2HeaderLength = WINTAP_FRAME_MINIMUM;
        packetRing->BeginIndex = NetRingIncrementIndex(
            packetRing,
            packetRing->BeginIndex);
        fragmentRing->BeginIndex = NetRingIncrementIndex(
            fragmentRing,
            fragmentRing->BeginIndex);
        ExFreePoolWithTag(frame, 'paTW');
    }

    if (releasedFrame) {
        WintapScheduleWriteDrain(ControlContext);
    }
}

_Use_decl_annotations_
VOID
WintapEvtPacketQueueStart(
    NETPACKETQUEUE Queue
    )
{
    InterlockedExchange(
        &WintapGetQueueContext(Queue)->Started,
        TRUE);
}

_Use_decl_annotations_
VOID
WintapEvtPacketQueueStop(
    NETPACKETQUEUE Queue
    )
{
    InterlockedExchange(
        &WintapGetQueueContext(Queue)->Started,
        FALSE);
}

_Use_decl_annotations_
VOID
WintapEvtPacketQueueAdvance(
    NETPACKETQUEUE Queue
    )
{
    PWINTAP_QUEUE_CONTEXT context = WintapGetQueueContext(Queue);
    PWINTAP_DEVICE_CONTEXT controlContext = WintapAcquireControlCallback();
    if (controlContext == nullptr) {
        return;
    }
    if (InterlockedCompareExchange(&context->Started, 0, 0) == 0) {
        WintapReleaseCallback(controlContext);
        return;
    }
    if (context->IsTransmit) {
        WintapCaptureTransmitPackets(context, controlContext);
    } else {
        WintapInjectReceiveFrames(context, controlContext);
    }
    WintapReleaseCallback(controlContext);
}

_Use_decl_annotations_
VOID
WintapEvtPacketQueueSetNotificationEnabled(
    NETPACKETQUEUE Queue,
    BOOLEAN NotificationEnabled
    )
{
    UNREFERENCED_PARAMETER(Queue);
    UNREFERENCED_PARAMETER(NotificationEnabled);
}

_Use_decl_annotations_
VOID
WintapEvtPacketQueueCancel(
    NETPACKETQUEUE Queue
    )
{
    UNREFERENCED_PARAMETER(Queue);
}
