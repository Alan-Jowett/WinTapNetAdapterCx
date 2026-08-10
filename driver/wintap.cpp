#include "wintap.h"

static constexpr ULONG WINTAP_FRAME_MINIMUM = 14;
static constexpr ULONG WINTAP_FRAME_MAXIMUM = 1514;
static constexpr ULONG WINTAP_FRAME_LIMIT = 256;
static PWINTAP_DEVICE_CONTEXT g_ControlContext = nullptr;
static WDFDEVICE g_ControlDevice = nullptr;
static NETADAPTER g_Adapter = nullptr;

static BOOLEAN
WintapAcquireCallback(
    _In_ PWINTAP_DEVICE_CONTEXT Context
    )
{
    BOOLEAN acquired = FALSE;
    WdfSpinLockAcquire(Context->FrameLock);
    if (!Context->Closing && !Context->Removed) {
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
    if (Context->Closing || *Count >= Context->FrameLimit) {
        WdfSpinLockRelease(Context->FrameLock);
        ExFreePoolWithTag(frame, 'paTW');
        return Context->Closing ? STATUS_DEVICE_NOT_READY : STATUS_DEVICE_BUSY;
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
        } else {
            WdfRequestComplete(request, status);
        }

        ExFreePoolWithTag(frame, 'paTW');
    }
}

static NTSTATUS
WintapProcessWriteRequest(
    _In_ PWINTAP_DEVICE_CONTEXT Context,
    _In_ WDFREQUEST Request,
    _Out_ size_t* BytesAccepted
    )
{
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

        size_t bytesAccepted = 0;
        status = WintapProcessWriteRequest(Context, request, &bytesAccepted);
        if (status == STATUS_DEVICE_BUSY) {
            status = WdfRequestForwardToIoQueue(request, Context->WriteQueue);
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
        enqueue = TRUE;
    }
    WdfSpinLockRelease(Context->FrameLock);

    if (enqueue) {
        WdfWorkItemEnqueue(Context->WriteDrainWorkItem);
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
    context->Closing = FALSE;
    context->Removed = FALSE;
    context->ActiveCallbacks = 0;
    KeInitializeEvent(&context->CallbackIdle, NotificationEvent, TRUE);
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
    WDF_OBJECT_ATTRIBUTES readAttributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&readAttributes);
    status = WdfIoQueueCreate(device, &readConfig, &readAttributes, &context->ReadQueue);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    WDF_IO_QUEUE_CONFIG writeConfig;
    WDF_IO_QUEUE_CONFIG_INIT(&writeConfig, WdfIoQueueDispatchManual);
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

    status = WdfDeviceCreateSymbolicLink(device, &symbolicLink);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    WdfControlFinishInitializing(device);
    g_ControlDevice = device;
    g_ControlContext = context;
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
        g_Adapter = adapter;
    }

    return status;
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
    WdfDeviceInitSetPnpPowerEventCallbacks(DeviceInit, &callbacks);

    WDFDEVICE device = nullptr;
    status = WdfDeviceCreate(&DeviceInit, WDF_NO_OBJECT_ATTRIBUTES, &device);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    status = WintapCreateAdapter(device);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    status = WintapCreateControlDevice(WdfDeviceGetDriver(device));
    if (!NT_SUCCESS(status) && g_Adapter != nullptr) {
        NetAdapterStop(g_Adapter);
        g_Adapter = nullptr;
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

    PWINTAP_DEVICE_CONTEXT context = g_ControlContext;
    if (context == nullptr) {
        return STATUS_SUCCESS;
    }

    WdfSpinLockAcquire(context->FrameLock);
    context->Removed = TRUE;
    context->Closing = TRUE;
    WdfSpinLockRelease(context->FrameLock);
    g_ControlContext = nullptr;
    if (g_Adapter != nullptr) {
        NetAdapterStop(g_Adapter);
        g_Adapter = nullptr;
    }
    KeWaitForSingleObject(
        &context->CallbackIdle,
        Executive,
        KernelMode,
        FALSE,
        nullptr);
    if (g_ControlDevice != nullptr) {
        WdfIoQueuePurgeSynchronously(context->ReadQueue);
        WdfIoQueuePurgeSynchronously(context->WriteQueue);
    }
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
    g_ControlContext = context;
    UNREFERENCED_PARAMETER(FileObject);
    WdfRequestComplete(Request, STATUS_SUCCESS);
}

_Use_decl_annotations_
VOID
WintapEvtFileCleanup(
    WDFFILEOBJECT FileObject
    )
{
    WDFDEVICE device = WdfFileObjectGetDevice(FileObject);
    PWINTAP_DEVICE_CONTEXT context = WintapGetDeviceContext(device);
    WdfSpinLockAcquire(context->FrameLock);
    context->Closing = TRUE;
    WdfSpinLockRelease(context->FrameLock);
    WdfIoQueuePurgeSynchronously(context->ReadQueue);
    WdfIoQueuePurgeSynchronously(context->WriteQueue);
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
    WdfSpinLockRelease(context->FrameLock);
    WintapDrainPendingWrites(context);
}

_Use_decl_annotations_
VOID
WintapEvtRequestCancel(
    WDFREQUEST Request
    )
{
    WdfRequestComplete(Request, STATUS_CANCELLED);
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
    PWINTAP_FRAME frame = WintapDequeueFrame(
        context,
        &context->FromStackQueue,
        &context->FromStackCount);
    if (frame == nullptr) {
        NTSTATUS status = WdfRequestForwardToIoQueue(Request, context->ReadQueue);
        if (!NT_SUCCESS(status)) {
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
    size_t bytesAccepted = 0;
    NTSTATUS status = WintapProcessWriteRequest(
        context,
        Request,
        &bytesAccepted);
    if (status == STATUS_DEVICE_BUSY) {
        status = WdfRequestForwardToIoQueue(Request, context->WriteQueue);
        if (!NT_SUCCESS(status)) {
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

    while (packetRing->BeginIndex != packetRing->EndIndex) {
        UINT32 packetIndex = packetRing->BeginIndex;
        NET_PACKET* packet = NetRingGetPacketAtIndex(packetRing, packetIndex);
        SIZE_T length = 0;
        UINT32 fragmentIndex = packet->FragmentIndex;
        for (UINT16 i = 0; i < packet->FragmentCount; ++i) {
            NET_FRAGMENT* fragment = NetRingGetFragmentAtIndex(
                fragmentRing,
                fragmentIndex);
            length += fragment->ValidLength;
            fragmentIndex = NetRingIncrementIndex(fragmentRing, fragmentIndex);
        }

        if (length >= WINTAP_FRAME_MINIMUM &&
            length <= WINTAP_FRAME_MAXIMUM) {
            SIZE_T allocationSize = FIELD_OFFSET(WINTAP_FRAME, Data) + length;
            PWINTAP_FRAME frame = static_cast<PWINTAP_FRAME>(
                ExAllocatePool2(POOL_FLAG_NON_PAGED, allocationSize, 'paTW'));
            if (frame != nullptr) {
                SIZE_T copied = 0;
                fragmentIndex = packet->FragmentIndex;
                for (UINT16 i = 0; i < packet->FragmentCount; ++i) {
                    NET_FRAGMENT* fragment = NetRingGetFragmentAtIndex(
                        fragmentRing,
                        fragmentIndex);
                    NET_FRAGMENT_VIRTUAL_ADDRESS* address =
                        NetExtensionGetFragmentVirtualAddress(
                            &QueueContext->FragmentVirtualAddress,
                            fragmentIndex);
                    SIZE_T fragmentLength = fragment->ValidLength;
                    RtlCopyMemory(
                        frame->Data + copied,
                        static_cast<UCHAR*>(address->VirtualAddress) +
                            fragment->Offset,
                        fragmentLength);
                    copied += fragmentLength;
                    fragmentIndex = NetRingIncrementIndex(
                        fragmentRing,
                        fragmentIndex);
                }
                frame->Length = copied;

                WdfSpinLockAcquire(ControlContext->FrameLock);
                if (!ControlContext->Closing &&
                    ControlContext->FromStackCount <
                        ControlContext->FrameLimit) {
                    InsertTailList(
                        &ControlContext->FromStackQueue,
                        &frame->ListEntry);
                    ++ControlContext->FromStackCount;
                    frame = nullptr;
                }
                WdfSpinLockRelease(ControlContext->FrameLock);
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

    WintapCompletePendingReads(ControlContext);
}

static VOID
WintapInjectReceiveFrames(
    _In_ PWINTAP_QUEUE_CONTEXT QueueContext,
    _In_ PWINTAP_DEVICE_CONTEXT ControlContext
    )
{
    NET_RING* packetRing = NetRingCollectionGetPacketRing(QueueContext->Rings);
    NET_RING* fragmentRing = NetRingCollectionGetFragmentRing(QueueContext->Rings);
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
        if (frame->Length > fragment->Capacity) {
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
    WintapGetQueueContext(Queue)->Started = TRUE;
}

_Use_decl_annotations_
VOID
WintapEvtPacketQueueStop(
    NETPACKETQUEUE Queue
    )
{
    WintapGetQueueContext(Queue)->Started = FALSE;
}

_Use_decl_annotations_
VOID
WintapEvtPacketQueueAdvance(
    NETPACKETQUEUE Queue
    )
{
    PWINTAP_DEVICE_CONTEXT controlContext = g_ControlContext;
    if (controlContext == nullptr ||
        controlContext->FrameLock == nullptr) {
        return;
    }

    PWINTAP_QUEUE_CONTEXT context = WintapGetQueueContext(Queue);
    if (!WintapAcquireCallback(controlContext)) {
        return;
    }
    if (!context->Started) {
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
