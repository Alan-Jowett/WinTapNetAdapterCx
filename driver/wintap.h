#pragma once

#include <ntddk.h>
#include <wdf.h>
#include <netadaptercx.h>
#include <net/virtualaddress.h>

typedef struct _WINTAP_FRAME {
    LIST_ENTRY ListEntry;
    SIZE_T Length;
    UCHAR Data[1];
} WINTAP_FRAME, *PWINTAP_FRAME;

typedef struct _WINTAP_DEVICE_CONTEXT {
    WDFQUEUE ReadQueue;
    WDFQUEUE WriteQueue;
    WDFQUEUE DefaultQueue;
    WDFWORKITEM WriteDrainWorkItem;
    WDFSPINLOCK FrameLock;
    LIST_ENTRY ToStackQueue;
    LIST_ENTRY FromStackQueue;
    ULONG ToStackCount;
    ULONG FromStackCount;
    ULONG FrameLimit;
    BOOLEAN Closing;
    BOOLEAN Removed;
    BOOLEAN WriteDrainQueued;
    LONG ActiveCallbacks;
    KEVENT CallbackIdle;
} WINTAP_DEVICE_CONTEXT, *PWINTAP_DEVICE_CONTEXT;

WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(WINTAP_DEVICE_CONTEXT, WintapGetDeviceContext);

typedef struct _WINTAP_QUEUE_CONTEXT {
    BOOLEAN IsTransmit;
    BOOLEAN Started;
    const NET_RING_COLLECTION* Rings;
    NET_EXTENSION FragmentVirtualAddress;
} WINTAP_QUEUE_CONTEXT, *PWINTAP_QUEUE_CONTEXT;

WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(WINTAP_QUEUE_CONTEXT, WintapGetQueueContext);

EXTERN_C_START

DRIVER_INITIALIZE DriverEntry;
EVT_WDF_DRIVER_DEVICE_ADD WintapEvtDeviceAdd;
EVT_WDF_DEVICE_PREPARE_HARDWARE WintapEvtPrepareHardware;
EVT_WDF_DEVICE_RELEASE_HARDWARE WintapEvtReleaseHardware;
EVT_WDF_DEVICE_FILE_CREATE WintapEvtFileCreate;
EVT_WDF_FILE_CLEANUP WintapEvtFileCleanup;
EVT_WDF_FILE_CLOSE WintapEvtFileClose;
EVT_WDF_IO_QUEUE_IO_READ WintapEvtIoRead;
EVT_WDF_IO_QUEUE_IO_WRITE WintapEvtIoWrite;
EVT_WDF_WORKITEM WintapEvtWriteDrainWorkItem;
EVT_WDF_REQUEST_CANCEL WintapEvtRequestCancel;
EVT_NET_ADAPTER_CREATE_TXQUEUE WintapEvtCreateTxQueue;
EVT_NET_ADAPTER_CREATE_RXQUEUE WintapEvtCreateRxQueue;
EVT_PACKET_QUEUE_ADVANCE WintapEvtPacketQueueAdvance;
EVT_PACKET_QUEUE_START WintapEvtPacketQueueStart;
EVT_PACKET_QUEUE_STOP WintapEvtPacketQueueStop;
EVT_PACKET_QUEUE_SET_NOTIFICATION_ENABLED WintapEvtPacketQueueSetNotificationEnabled;
EVT_PACKET_QUEUE_CANCEL WintapEvtPacketQueueCancel;

NTSTATUS
WintapCreateControlDevice(
    _In_ WDFDRIVER Driver
    );

EXTERN_C_END
