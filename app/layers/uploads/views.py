from fastapi import (
    Depends,
    APIRouter,
    Request,
    Query,
    BackgroundTasks,
    Header,
    Response,
    HTTPException,
)
from app.db import get_session, AsyncSession
from app.config import config
from uuid import UUID
from sqlmodel import select, update
from typing import Any, Annotated
import datetime

router = APIRouter()


@router.post("")
async def upload_file(
    request: Request,
    user_id: Annotated[UUID, Header()],
    upload_length: int = Header(..., alias="Upload-Length"),
    content_type: str = Header(..., alias="Content-Type"),
    transect_id: str = Header(None, alias="Transect-Id"),
    session: AsyncSession = Depends(get_session),
    *,
    background_tasks: BackgroundTasks,
) -> str:
    # Handle chunked file upload
    try:
        object = InputObject(
            size_bytes=upload_length,
            all_parts_received=False,
            last_part_received_utc=datetime.datetime.now(),
            processing_message="Upload started",
            transect_id=transect_id if transect_id else None,
            owner=user_id,
        )
        session.add(object)

        await session.commit()
        await session.refresh(object)

        key = f"{config.S3_PREFIX}/inputs/{str(object.id)}"
        # Create multipart upload and add the upload id to the object
        response = await s3.create_multipart_upload(
            Bucket=config.S3_BUCKET_ID,
            Key=key,
        )

        # Wait for the response to return the upload id
        object.upload_id = response["UploadId"]
        await session.commit()

        # Create a worker to monitor stale uploads and delete if outside
        # of threshold in config
        background_tasks.add_task(
            delete_incomplete_object, object.id, s3, session
        )

    except Exception as e:
        print(e)
        await session.rollback()
        raise HTTPException(
            status_code=500,
            detail="Failed to create object",
        )

    return str(object.id)


@router.patch("")
async def upload_chunk(
    request: Request,
    patch: str = Query(...),
    user_id: UUID | None = Header(...),
    user_is_admin: bool = Header(...),
    session: AsyncSession = Depends(get_session),
    upload_offset: int = Header(..., alias="Upload-Offset"),
    upload_length: int = Header(..., alias="Upload-Length"),
    upload_name: str = Header(..., alias="Upload-Name"),
    content_type: str = Header(..., alias="Content-Type"),
    content_length: int = Header(..., alias="Content-Length"),
    *,
    background_tasks: BackgroundTasks,
):
    """Handle chunked file upload"""

    # Clean " from patch
    patch = patch.replace('"', "")

    # Get the object prefix from the DB
    query = select(InputObject).where(InputObject.id == patch)
    if not user_is_admin:
        query = query.where(InputObject.owner == user_id)
    res = await session.exec(query)
    object = res.one_or_none()
    if not object:
        raise HTTPException(
            status_code=404,
            detail="Object not found",
        )

    final_part = False
    # Get the part number from the offset
    if upload_length - upload_offset == content_length:
        # The last object is probably not the same size as the other parts, so
        # we need to check if the part number is the last part
        last_part = object.parts[-1]
        part_number = last_part["PartNumber"] + 1
    else:
        # Calculate the part number from the division of the offset by the part
        # size (content-length) from the upload length
        part_number = (int(upload_offset) // int(content_length)) + 1

    if upload_offset + content_length == upload_length:
        final_part = True

    # Upload the chunk to S3
    key = f"{config.S3_PREFIX}/inputs/{str(object.id)}"

    data = await request.body()

    print(
        f"Working on part number {part_number}, chunk {upload_offset} "
        f"{int(upload_offset)+int(content_length)} "
        f"of {upload_length} bytes "
        f"({int(upload_offset)/int(upload_length)*100}%)"
    )
    try:
        part = await s3.upload_part(
            Bucket=config.S3_BUCKET_ID,
            Key=key,
            UploadId=object.upload_id,
            PartNumber=part_number,
            Body=data,
        )
        if object.parts is None or not object.parts:
            object.parts = []

        object.parts += [
            {
                "PartNumber": part_number,
                "ETag": part["ETag"],
                "Size": content_length,
                "Offset": upload_offset,
                "Length": content_length,
            }
        ]
        update_query = (
            update(InputObject)
            .where(InputObject.id == object.id)
            .values(
                parts=object.parts,
                filename=upload_name,
                last_part_received_utc=datetime.datetime.now(),
            )
        )
        await session.exec(update_query)
        await session.commit()

        if final_part:
            # Complete the multipart upload

            # Simplify the parts list to only include the PartNumber and ETag
            parts_list = [
                {"PartNumber": x["PartNumber"], "ETag": x["ETag"]}
                for x in object.parts
            ]
            res = await s3.complete_multipart_upload(
                Bucket=config.S3_BUCKET_ID,
                Key=key,
                UploadId=object.upload_id,
                MultipartUpload={"Parts": parts_list},
            )

            update_query = (
                update(InputObject)
                .where(InputObject.id == object.id)
                .values(
                    last_part_received_utc=datetime.datetime.now(),
                    all_parts_received=True,
                )
            )
            await session.exec(update_query)
            await session.commit()

            # Create background task to generate video statistics
            background_tasks.add_task(
                generate_video_statistics, object.id, s3, session
            )

    except Exception as e:
        print(e)
        raise HTTPException(
            status_code=500,
            detail=f"Failed to upload file to S3: {e}",
        )

    await session.refresh(object)
    return object


@router.head("")
async def check_uploaded_chunks(
    response: Response,
    user_id: UUID | None = Header(...),
    user_is_admin: bool = Header(...),
    patch: str = Query(...),
    session: AsyncSession = Depends(get_session),
):
    """Responds with the offset of the next expected chunk"""
    # Get the InputObject from the DB
    # Clean " from patch
    patch = patch.replace('"', "")
    query = select(InputObject).where(InputObject.id == patch)
    if not user_is_admin:
        query = query.where(InputObject.owner == user_id)
    res = await session.exec(query)
    object = res.one_or_none()
    if not object:
        raise HTTPException(
            status_code=404,
            detail="Object not found",
        )

    # Calculate the next expected offset
    next_expected_offset = 0
    if object.parts:
        last_part = object.parts[-1]
        next_expected_offset = last_part["Offset"] + last_part["Length"]

    # Return headers with Upload-Offset
    response.headers["Upload-Offset"] = str(next_expected_offset)

    return
