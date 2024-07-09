from typing import Any
from fastapi import Depends, APIRouter, Query, Response
from app.config import config
from app.users.models import User
from app.auth import require_admin
from keycloak import KeycloakAdmin, KeycloakOpenIDConnection
from pydantic import BaseModel
import json


router = APIRouter()


def get_keycloak_admin():
    keycloak_connection = KeycloakOpenIDConnection(
        server_url=config.KEYCLOAK_URL,
        realm_name=config.KEYCLOAK_REALM,
        client_id=config.KEYCLOAK_API_ID,
        client_secret_key=config.KEYCLOAK_API_SECRET,
        verify=True,
    )
    return KeycloakAdmin(connection=keycloak_connection)


@router.get("")
async def get_users(
    response: Response,
    user: User = Depends(require_admin),
    keycloak_admin=Depends(get_keycloak_admin),
    *,
    filter: str = Query(None),
    sort: str = Query(None),
    range: str = Query(None),
) -> Any:
    """Get a list of users

    This endpoint is used to get a list of users from Keycloak. It is used
    to populate the list of users in the admin UI.

    Filtering by username is only supported when the admin filter is set to
    False.
    """

    sort: list[Any] = json.loads(sort) if sort else []
    range: list[Any] = json.loads(range) if range else []
    filter: dict[Any, Any] = json.loads(filter) if filter else {}

    # Get admin role users
    admin_users = keycloak_admin.get_realm_role_members("admin")

    for admin_user in admin_users:
        admin_user["admin"] = True

    query = {"first": range[0], "max": range[1] - range[0] + 1}

    # Manage the filter if there is one

    # If admin is False, then handle general users in Keycloak
    # Username filtering is only supported when admin is False, only
    # because we assume the list for admins is much smaller and we can
    # avoid querying the entire user list (all of EPFL LDAP in this case)
    # if "admin" in filter and filter["admin"] is False:
    if "username" in filter:
        query["username"] = filter["username"]

    users = keycloak_admin.get_users(query=query)

    for admin_user in users:
        if admin_user["username"] in [u["username"] for u in admin_users]:
            admin_user["admin"] = True
        else:
            admin_user["admin"] = False

    if "admin" in filter and filter["admin"] is True:
        # Otherwise, return only the admin users
        users = admin_users

    if len(range) == 2:
        start, end = range
    else:
        start, end = [0, len(users) + 1]

    response.headers["Content-Range"] = f"users {start}-{end}/{len(users)}"

    return users[start : end + 1]


class UserUpdate(BaseModel):
    admin: bool


@router.put("/{user_id}")
async def update_user(
    user_id: str,
    user_update: UserUpdate,
    keycloak_admin: KeycloakAdmin = Depends(get_keycloak_admin),
    user: User = Depends(require_admin),
) -> Any:
    """Updates the role of the user"""

    # Get roles
    realm_roles = keycloak_admin.get_realm_roles(search_text="admin")

    if len(realm_roles) == 0:
        raise Exception("No admin role found")
    if len(realm_roles) > 1:
        raise Exception("Multiple admin roles found")
    admin_realm_role = realm_roles[0]

    if user_update.admin:
        keycloak_admin.assign_realm_roles(
            user_id=user_id,
            roles=admin_realm_role,
        )
    else:
        keycloak_admin.delete_realm_roles_of_user(
            user_id=user_id,
            roles=admin_realm_role,
        )

    return keycloak_admin.get_user(user_id)
